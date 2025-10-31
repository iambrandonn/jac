//! Parallel processing support utilities.
//!
//! Phase 2 introduces automatic decision logic for selecting between the
//! sequential and parallel compression paths. Later phases will add the
//! actual parallel pipeline that consumes this decision.

use crate::InputSource;
use jac_format::{Limits, Result};

#[cfg(not(target_arch = "wasm32"))]
use sysinfo::System;

const SMALL_FILE_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
const MEMORY_RESERVATION_NUMERATOR: u64 = 3;
const MEMORY_RESERVATION_DENOMINATOR: u64 = 4;
const MEMORY_PER_THREAD_MULTIPLIER: u64 = 2;
const MAX_PARALLEL_THREADS: usize = 16;

/// Decision returned by the parallel heuristic describing whether parallel
/// compression should be enabled along with diagnostic metadata.
#[derive(Debug, Clone)]
pub struct ParallelDecision {
    /// Whether the parallel pipeline should be used.
    pub use_parallel: bool,
    /// Number of worker threads to use when parallelism is enabled.
    pub thread_count: usize,
    /// Human-readable explanation of the chosen mode.
    pub reason: String,
    /// Estimated peak memory consumption in bytes.
    pub estimated_memory: u64,
    /// Available memory reported by the system in bytes.
    pub available_memory: u64,
}

/// Determine whether parallel compression should be used for the provided
/// input. On non-WASM targets this consults CPU and memory availability;
/// WASM targets always fall back to sequential execution.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn should_use_parallel(
    input_source: &InputSource,
    limits: &Limits,
) -> Result<ParallelDecision> {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let mut system = System::new();
    system.refresh_memory();
    let available_memory_bytes = system.available_memory().saturating_mul(1024);

    let input_size_hint = match input_source {
        InputSource::NdjsonPath(path) | InputSource::JsonArrayPath(path) => {
            std::fs::metadata(path).map(|meta| meta.len()).ok()
        }
        InputSource::NdjsonReader(_)
        | InputSource::JsonArrayReader(_)
        | InputSource::Iterator(_) => None,
    };

    Ok(evaluate_parallel_decision(
        cores,
        available_memory_bytes,
        limits.max_block_uncompressed_total,
        input_size_hint,
    ))
}

/// WASM targets default to sequential compression to avoid depending on
/// unavailable system APIs and threading primitives.
#[cfg(target_arch = "wasm32")]
pub(crate) fn should_use_parallel(
    _input_source: &InputSource,
    _limits: &Limits,
) -> Result<ParallelDecision> {
    Ok(ParallelDecision {
        use_parallel: false,
        thread_count: 1,
        reason: "WASM target does not support parallelism".into(),
        estimated_memory: 0,
        available_memory: 0,
    })
}

fn evaluate_parallel_decision(
    cores: usize,
    available_memory_bytes: u64,
    max_block_uncompressed_total: usize,
    input_size_hint: Option<u64>,
) -> ParallelDecision {
    if cores < 2 {
        return ParallelDecision {
            use_parallel: false,
            thread_count: 1,
            reason: "Single-core system detected".into(),
            estimated_memory: 0,
            available_memory: available_memory_bytes,
        };
    }

    let per_block_memory = std::cmp::max(max_block_uncompressed_total as u64, 1u64);
    let memory_per_thread = std::cmp::max(
        per_block_memory.saturating_mul(MEMORY_PER_THREAD_MULTIPLIER),
        1u64,
    );

    let usable_memory = available_memory_bytes.saturating_mul(MEMORY_RESERVATION_NUMERATOR)
        / MEMORY_RESERVATION_DENOMINATOR;

    let raw_safe_threads = if memory_per_thread == 0 {
        0
    } else {
        (usable_memory / memory_per_thread) as usize
    };
    let max_safe_threads = std::cmp::max(raw_safe_threads, 1);

    let thread_count = std::cmp::max(
        1,
        std::cmp::min(cores, std::cmp::min(max_safe_threads, MAX_PARALLEL_THREADS)),
    );

    if thread_count < 2 {
        let estimated_memory = memory_per_thread.saturating_mul(thread_count as u64);
        return ParallelDecision {
            use_parallel: false,
            thread_count: 1,
            reason: format!(
                "Insufficient memory for parallel compression: {} cores available, but only enough memory for {} threads ({:.1} MiB available, {:.1} MiB per thread required)",
                cores,
                max_safe_threads,
                bytes_to_mib(available_memory_bytes),
                bytes_to_mib(memory_per_thread),
            ),
            estimated_memory,
            available_memory: available_memory_bytes,
        };
    }

    if let Some(size) = input_size_hint {
        if size > 0 && size < SMALL_FILE_THRESHOLD_BYTES {
            return ParallelDecision {
                use_parallel: false,
                thread_count: 1,
                reason: format!(
                    "Small input file ({:.1} MiB) - parallel overhead exceeds benefit",
                    bytes_to_mib(size),
                ),
                estimated_memory: per_block_memory,
                available_memory: available_memory_bytes,
            };
        }
    }

    let estimated_memory = memory_per_thread.saturating_mul(thread_count as u64);

    ParallelDecision {
        use_parallel: true,
        thread_count,
        reason: format!(
            "Using {}/{} cores for parallel compression ({:.1} MiB estimated peak memory)",
            thread_count,
            cores,
            bytes_to_mib(estimated_memory),
        ),
        estimated_memory,
        available_memory: available_memory_bytes,
    }
}

fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jac_format::Limits;
    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn decision_respects_memory_cap() {
        let decision = evaluate_parallel_decision(
            8,
            4 * GIB,
            Limits::default().max_block_uncompressed_total,
            None,
        );

        assert!(decision.use_parallel);
        assert_eq!(decision.thread_count, 6);
    }

    #[test]
    fn single_core_forces_sequential() {
        let decision = evaluate_parallel_decision(
            1,
            16 * 1024 * 1024 * 1024,
            Limits::default().max_block_uncompressed_total,
            None,
        );

        assert!(!decision.use_parallel);
        assert_eq!(decision.thread_count, 1);
    }

    #[test]
    fn small_file_prefers_sequential() {
        let decision = evaluate_parallel_decision(
            8,
            16 * 1024 * 1024 * 1024,
            Limits::default().max_block_uncompressed_total,
            Some(5 * 1024 * 1024),
        );
        assert!(!decision.use_parallel);
        assert_eq!(decision.thread_count, 1);
    }

    #[test]
    fn iterator_default_parallel_when_resources_allow() {
        let decision = evaluate_parallel_decision(
            8,
            16 * GIB,
            Limits::default().max_block_uncompressed_total,
            None,
        );
        assert!(decision.use_parallel);
        assert!(decision.thread_count >= 2);
    }

    #[test]
    fn low_memory_caps_threads() {
        let decision = evaluate_parallel_decision(16, 2 * GIB, 256 * 1024 * 1024, None);
        assert!(decision.use_parallel);
        assert_eq!(decision.thread_count, 3);
    }

    #[test]
    fn large_file_prefers_parallel() {
        let decision = evaluate_parallel_decision(
            12,
            32 * GIB,
            Limits::default().max_block_uncompressed_total,
            Some(50 * 1024 * 1024),
        );
        assert!(decision.use_parallel);
        assert_eq!(decision.thread_count, 12.min(MAX_PARALLEL_THREADS));
    }

    #[test]
    fn insufficient_memory_forces_sequential() {
        let decision = evaluate_parallel_decision(
            8,
            128 * 1024 * 1024, // 128 MiB total available
            256 * 1024 * 1024, // 256 MiB per block
            None,
        );
        assert!(!decision.use_parallel);
        assert_eq!(decision.thread_count, 1);
    }
}
