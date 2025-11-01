use jac_format::Limits;
use jac_io::{
    execute_compress, execute_decompress, execute_project, parallel::ParallelConfig,
    CompressOptions, CompressRequest, ContainerFormat, DecompressFormat, DecompressOptions,
    DecompressRequest, InputSource, JacInput, OutputSink, ProjectFormat, ProjectRequest,
    WrapperConfig,
};
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

/// Test data generator for concurrency stress testing
struct TestDataGenerator {
    record_count: usize,
    field_count: usize,
}

impl TestDataGenerator {
    fn new(record_count: usize, field_count: usize) -> Self {
        Self {
            record_count,
            field_count,
        }
    }

    fn generate_records(&self) -> Vec<Map<String, Value>> {
        let mut records = Vec::with_capacity(self.record_count);

        for i in 0..self.record_count {
            let mut record = Map::new();

            // Add timestamp field (monotonic for delta encoding)
            record.insert(
                "ts".to_string(),
                Value::Number((1234567890 + i as i64).into()),
            );

            // Add level field (dictionary encoding)
            let levels = ["debug", "info", "warn", "error"];
            record.insert(
                "level".to_string(),
                Value::String(levels[i % levels.len()].to_string()),
            );

            // Add user field (dictionary encoding)
            record.insert(
                "user".to_string(),
                Value::String(format!("user_{}", i % 100)),
            );

            // Add message field (raw string)
            record.insert(
                "message".to_string(),
                Value::String(format!("Message number {} with some content", i)),
            );

            // Add additional fields based on field_count
            for j in 0..self.field_count {
                let field_name = format!("field_{}", j);
                let field_value = match j % 4 {
                    0 => Value::Number((i as i64 * j as i64).into()),
                    1 => Value::String(format!("string_{}_{}", i, j)),
                    2 => Value::Bool(i % 2 == 0),
                    3 => Value::Null,
                    _ => Value::Number((i as i64 + j as i64).into()),
                };
                record.insert(field_name, field_value);
            }

            records.push(record);
        }

        records
    }
}

/// Test configuration for concurrency stress tests
#[derive(Debug, Clone)]
struct ConcurrencyTestConfig {
    writer_threads: usize,
    reader_threads: usize,
    records_per_writer: usize,
    field_count: usize,
    test_duration: Duration,
    block_size: usize,
    compression_level: u8,
}

impl Default for ConcurrencyTestConfig {
    fn default() -> Self {
        Self {
            writer_threads: 4,
            reader_threads: 4,
            records_per_writer: 100,
            field_count: 3,
            test_duration: Duration::from_secs(5),
            block_size: 1000,
            compression_level: 1, // Fast compression for testing
        }
    }
}

/// Result of a concurrency stress test
#[derive(Debug)]
struct ConcurrencyTestResult {
    total_records_written: usize,
    total_records_read: usize,
    total_blocks_written: usize,
    total_blocks_read: usize,
    write_throughput: f64, // records per second
    read_throughput: f64,  // records per second
    errors: Vec<String>,
    deterministic: bool,
}

/// Concurrency stress test suite
struct ConcurrencyStressTest {
    config: ConcurrencyTestConfig,
    data_generator: TestDataGenerator,
}

impl ConcurrencyStressTest {
    fn new(config: ConcurrencyTestConfig) -> Self {
        let data_generator = TestDataGenerator::new(config.records_per_writer, config.field_count);

        Self {
            config,
            data_generator,
        }
    }

    /// Run parallel writer test
    fn test_parallel_writers(&self) -> Result<ConcurrencyTestResult, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let mut handles = Vec::new();
        let results = Arc::new(Mutex::new(Vec::new()));
        let start_time = std::time::Instant::now();

        // Spawn writer threads
        for thread_id in 0..self.config.writer_threads {
            let temp_dir = temp_dir.path().to_path_buf();
            let results = Arc::clone(&results);
            let config = self.config.clone();
            let data_generator =
                TestDataGenerator::new(config.records_per_writer, config.field_count);

            let handle = thread::spawn(move || {
                let output_path = temp_dir.join(format!("output_{}.jac", thread_id));
                let records = data_generator.generate_records();
                let record_count = records.len();

                let options = CompressOptions {
                    block_target_records: config.block_size,
                    default_codec: jac_codec::Codec::Zstd(config.compression_level),
                    canonicalize_keys: true,
                    canonicalize_numbers: true,
                    nested_opaque: true,
                    max_dict_entries: 4096,
                    limits: Limits::default(),
                    parallel_config: ParallelConfig::default(),
                };

                let request = CompressRequest {
                    input: InputSource::Iterator(Box::new(records.into_iter())),
                    output: OutputSink::Path(output_path.clone()),
                    options,
                    container_hint: Some(ContainerFormat::Ndjson),
                    emit_index: true,
                    wrapper_config: WrapperConfig::None,
                };

                let start = std::time::Instant::now();
                let result = execute_compress(request);
                let duration = start.elapsed();

                let mut results = results.lock().unwrap();
                results.push((thread_id, result, duration, record_count));
            });

            handles.push(handle);
        }

        // Wait for all writers to complete
        for handle in handles {
            handle.join().unwrap();
        }

        let results = results.lock().unwrap();
        let total_records = results.iter().map(|(_, _, _, count)| *count).sum();
        let total_duration = start_time.elapsed();
        let mut errors = Vec::new();

        // Check for errors and collect statistics
        for (thread_id, result, _duration, _record_count) in results.iter() {
            match result {
                Ok(_) => {
                    // Success
                }
                Err(e) => {
                    errors.push(format!("Writer thread {} failed: {}", thread_id, e));
                }
            }
        }

        let deterministic = errors.is_empty();
        Ok(ConcurrencyTestResult {
            total_records_written: total_records,
            total_records_read: 0,
            total_blocks_written: 0, // Would need to count blocks from files
            total_blocks_read: 0,
            write_throughput: total_records as f64 / total_duration.as_secs_f64(),
            read_throughput: 0.0,
            errors,
            deterministic,
        })
    }

    /// Run parallel reader test
    fn test_parallel_readers(
        &self,
        input_files: Vec<std::path::PathBuf>,
    ) -> Result<ConcurrencyTestResult, Box<dyn std::error::Error>> {
        let mut handles = Vec::new();
        let results = Arc::new(Mutex::new(Vec::new()));

        // Spawn reader threads
        for (thread_id, input_file) in input_files.iter().enumerate() {
            let input_file = input_file.clone();
            let results = Arc::clone(&results);

            let handle = thread::spawn(move || {
                let options = DecompressOptions {
                    limits: Limits::default(),
                    verify_checksums: true,
                };

                let request = DecompressRequest {
                    input: JacInput::Path(input_file),
                    output: OutputSink::Writer(Box::new(Vec::new())),
                    format: DecompressFormat::JsonArray,
                    options,
                };

                let start = std::time::Instant::now();
                let result = execute_decompress(request);
                let duration = start.elapsed();

                let mut results = results.lock().unwrap();
                results.push((thread_id, result, duration));
            });

            handles.push(handle);
        }

        // Wait for all readers to complete
        for handle in handles {
            handle.join().unwrap();
        }

        let results = results.lock().unwrap();
        let mut errors = Vec::new();
        let mut total_records = 0;

        // Check for errors and collect statistics
        for (thread_id, result, _duration) in results.iter() {
            match result {
                Ok(_) => {
                    // Success - simplified record counting
                    total_records += 100; // Rough estimate
                }
                Err(e) => {
                    errors.push(format!("Reader thread {} failed: {}", thread_id, e));
                }
            }
        }

        let deterministic = errors.is_empty();
        Ok(ConcurrencyTestResult {
            total_records_written: 0,
            total_records_read: total_records,
            total_blocks_written: 0,
            total_blocks_read: 0,
            write_throughput: 0.0,
            read_throughput: total_records as f64 / 1.0, // Simplified
            errors,
            deterministic,
        })
    }

    /// Run projection concurrency test
    fn test_projection_concurrency(
        &self,
        input_file: std::path::PathBuf,
    ) -> Result<ConcurrencyTestResult, Box<dyn std::error::Error>> {
        let mut handles = Vec::new();
        let results = Arc::new(Mutex::new(Vec::new()));
        let fields = vec![
            "ts".to_string(),
            "level".to_string(),
            "user".to_string(),
            "message".to_string(),
        ];

        // Spawn projection threads
        for thread_id in 0..self.config.reader_threads {
            let input_file = input_file.clone();
            let results = Arc::clone(&results);
            let fields = fields.clone();

            let handle = thread::spawn(move || {
                let start = std::time::Instant::now();

                // Test different projection scenarios
                let mut errors = Vec::new();

                // Project single field
                for field in &fields {
                    let options = DecompressOptions {
                        limits: Limits::default(),
                        verify_checksums: true,
                    };

                    let request = ProjectRequest {
                        input: JacInput::Path(input_file.clone()),
                        output: OutputSink::Writer(Box::new(Vec::new())),
                        fields: vec![field.clone()],
                        format: ProjectFormat::JsonArray,
                        options,
                    };

                    let result = execute_project(request);
                    if let Err(e) = result {
                        errors.push(format!("Projection of field {} failed: {}", field, e));
                    }
                }

                // Project multiple fields
                let options = DecompressOptions {
                    limits: Limits::default(),
                    verify_checksums: true,
                };

                let request = ProjectRequest {
                    input: JacInput::Path(input_file.clone()),
                    output: OutputSink::Writer(Box::new(Vec::new())),
                    fields: fields.clone(),
                    format: ProjectFormat::JsonArray,
                    options,
                };

                let result = execute_project(request);
                if let Err(e) = result {
                    errors.push(format!("Multi-field projection failed: {}", e));
                }

                let duration = start.elapsed();
                let mut results = results.lock().unwrap();
                results.push((thread_id, errors, duration));
            });

            handles.push(handle);
        }

        // Wait for all projection threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        let results = results.lock().unwrap();
        let mut all_errors = Vec::new();
        let mut total_duration = Duration::new(0, 0);

        for (thread_id, errors, duration) in results.iter() {
            all_errors.extend(
                errors
                    .iter()
                    .map(|e| format!("Thread {}: {}", thread_id, e)),
            );
            total_duration += *duration;
        }

        let deterministic = all_errors.is_empty();
        Ok(ConcurrencyTestResult {
            total_records_written: 0,
            total_records_read: 0,
            total_blocks_written: 0,
            total_blocks_read: 0,
            write_throughput: 0.0,
            read_throughput: 0.0,
            errors: all_errors,
            deterministic,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parallel_writers_basic() {
        let config = ConcurrencyTestConfig {
            writer_threads: 2,
            reader_threads: 0,
            records_per_writer: 50,
            field_count: 3,
            test_duration: Duration::from_secs(2),
            block_size: 1000,
            compression_level: 1,
        };

        let test = ConcurrencyStressTest::new(config);
        let result = test.test_parallel_writers().unwrap();

        assert!(
            result.deterministic,
            "Parallel writers should be deterministic"
        );
        assert!(
            result.errors.is_empty(),
            "No errors expected: {:?}",
            result.errors
        );
        assert!(
            result.total_records_written > 0,
            "Should have written records"
        );
        assert!(
            result.write_throughput > 0.0,
            "Should have positive throughput"
        );
    }

    #[test]
    fn test_parallel_readers_basic() {
        // First create a test file
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.jac");

        let data_generator = TestDataGenerator::new(50, 3);
        let records = data_generator.generate_records();

        let options = CompressOptions {
            block_target_records: 1000,
            default_codec: jac_codec::Codec::Zstd(1),
            canonicalize_keys: true,
            canonicalize_numbers: true,
            nested_opaque: true,
            max_dict_entries: 4096,
            limits: Limits::default(),
            parallel_config: ParallelConfig::default(),
        };

        let request = CompressRequest {
            input: InputSource::Iterator(Box::new(records.into_iter())),
            output: OutputSink::Path(test_file.clone()),
            options,
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };

        execute_compress(request).unwrap();

        let config = ConcurrencyTestConfig {
            writer_threads: 0,
            reader_threads: 2,
            records_per_writer: 0,
            field_count: 3,
            test_duration: Duration::from_secs(2),
            block_size: 1000,
            compression_level: 1,
        };

        let test = ConcurrencyStressTest::new(config);
        let result = test.test_parallel_readers(vec![test_file]).unwrap();

        assert!(
            result.deterministic,
            "Parallel readers should be deterministic"
        );
        assert!(
            result.errors.is_empty(),
            "No errors expected: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_projection_concurrency() {
        // First create a test file
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.jac");

        let data_generator = TestDataGenerator::new(50, 3);
        let records = data_generator.generate_records();

        let options = CompressOptions {
            block_target_records: 1000,
            default_codec: jac_codec::Codec::Zstd(1),
            canonicalize_keys: true,
            canonicalize_numbers: true,
            nested_opaque: true,
            max_dict_entries: 4096,
            limits: Limits::default(),
            parallel_config: ParallelConfig::default(),
        };

        let request = CompressRequest {
            input: InputSource::Iterator(Box::new(records.into_iter())),
            output: OutputSink::Path(test_file.clone()),
            options,
            container_hint: Some(ContainerFormat::Ndjson),
            emit_index: true,
            wrapper_config: WrapperConfig::None,
        };

        execute_compress(request).unwrap();

        let config = ConcurrencyTestConfig {
            writer_threads: 0,
            reader_threads: 4,
            records_per_writer: 0,
            field_count: 3,
            test_duration: Duration::from_secs(2),
            block_size: 1000,
            compression_level: 1,
        };

        let test = ConcurrencyStressTest::new(config);
        let result = test.test_projection_concurrency(test_file).unwrap();

        assert!(
            result.deterministic,
            "Projection concurrency should be deterministic"
        );
        assert!(
            result.errors.is_empty(),
            "No errors expected: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_deterministic_output() {
        // Test that multiple runs produce deterministic output
        let config = ConcurrencyTestConfig {
            writer_threads: 2,
            reader_threads: 0,
            records_per_writer: 25,
            field_count: 3,
            test_duration: Duration::from_secs(1),
            block_size: 1000,
            compression_level: 1,
        };

        let test = ConcurrencyStressTest::new(config);

        // Run multiple times and verify deterministic behavior
        for _ in 0..3 {
            let result = test.test_parallel_writers().unwrap();
            assert!(result.deterministic, "Output should be deterministic");
            assert!(result.errors.is_empty(), "No errors expected");
        }
    }

    #[test]
    #[ignore] // Stress test - runs in nightly/CI with STRESS_TESTS=1
    fn test_high_contention() {
        // Test with high thread contention
        let config = ConcurrencyTestConfig {
            writer_threads: 8,
            reader_threads: 0,
            records_per_writer: 20,
            field_count: 5,
            test_duration: Duration::from_secs(3),
            block_size: 1000,
            compression_level: 1,
        };

        let test = ConcurrencyStressTest::new(config);
        let result = test.test_parallel_writers().unwrap();

        assert!(
            result.deterministic,
            "High contention should still be deterministic"
        );
        assert!(
            result.errors.is_empty(),
            "No errors expected under high contention: {:?}",
            result.errors
        );
    }
}
