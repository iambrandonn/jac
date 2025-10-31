use crate::CompressionRuntimeStats;
use std::time::{Duration, Instant};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

#[cfg(not(target_arch = "wasm32"))]
use sysinfo::{get_current_pid, ProcessRefreshKind, RefreshKind, System};

/// Helper structure that measures wall-clock duration and optional peak RSS usage.
pub(crate) struct RuntimeMeasurement {
    start: Instant,
    #[cfg(not(target_arch = "wasm32"))]
    sampler: Option<MemorySampler>,
}

impl RuntimeMeasurement {
    /// Begin a new runtime measurement window.
    pub(crate) fn begin() -> Self {
        Self {
            start: Instant::now(),
            #[cfg(not(target_arch = "wasm32"))]
            sampler: MemorySampler::spawn(Duration::from_millis(50)),
        }
    }

    /// Finish the measurement window and emit runtime statistics.
    pub(crate) fn finish(mut self) -> CompressionRuntimeStats {
        #[cfg(not(target_arch = "wasm32"))]
        let peak_rss_bytes = self
            .sampler
            .as_mut()
            .map(|sampler| {
                sampler.stop();
                sampler.peak_bytes()
            })
            .filter(|bytes| *bytes > 0);

        #[cfg(target_arch = "wasm32")]
        let peak_rss_bytes = None;

        CompressionRuntimeStats {
            wall_time: self.start.elapsed(),
            peak_rss_bytes,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for RuntimeMeasurement {
    fn drop(&mut self) {
        if let Some(sampler) = self.sampler.as_mut() {
            sampler.stop();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct MemorySampler {
    stop_flag: Arc<AtomicBool>,
    peak_bytes: Arc<AtomicU64>,
    handle: Option<thread::JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl MemorySampler {
    fn spawn(interval: Duration) -> Option<Self> {
        let pid = get_current_pid().ok()?;
        let stop_flag = Arc::new(AtomicBool::new(false));
        let peak_bytes = Arc::new(AtomicU64::new(0));

        let thread_stop = Arc::clone(&stop_flag);
        let thread_peak = Arc::clone(&peak_bytes);

        let handle = thread::Builder::new()
            .name("jac-memory-sampler".to_string())
            .spawn(move || {
                let mut system = System::new_with_specifics(
                    RefreshKind::new().with_processes(ProcessRefreshKind::new()),
                );
                let refresh_kind = ProcessRefreshKind::new().with_memory();

                while !thread_stop.load(Ordering::Relaxed) {
                    if !system.refresh_process_specifics(pid, refresh_kind) {
                        // Fallback to refreshing the single process if specifics not supported.
                        system.refresh_process(pid);
                    }

                    if let Some(process) = system.process(pid) {
                        let rss_bytes = process.memory() as u64 * 1024;
                        thread_peak.fetch_max(rss_bytes, Ordering::Relaxed);
                    }

                    thread::sleep(interval);
                }
            })
            .ok()?;

        Some(Self {
            stop_flag,
            peak_bytes,
            handle: Some(handle),
        })
    }

    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    fn peak_bytes(&self) -> u64 {
        self.peak_bytes.load(Ordering::Relaxed)
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for MemorySampler {
    fn drop(&mut self) {
        self.stop();
    }
}
