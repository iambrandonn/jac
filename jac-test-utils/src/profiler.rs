//! Test performance profiler for JAC tests
//!
//! This module provides tools for profiling test execution and identifying
//! performance bottlenecks and resource usage patterns.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Performance profiler for test execution
pub struct TestProfiler {
    start_time: Instant,
    measurements: Vec<PerformanceMeasurement>,
    current_measurement: Option<String>,
}

/// Individual performance measurement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMeasurement {
    pub name: String,
    pub duration: Duration,
    pub memory_before: Option<u64>,
    pub memory_after: Option<u64>,
    pub memory_delta: Option<i64>,
    pub cpu_usage: Option<f64>,
    pub io_operations: Option<u64>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Performance analysis results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceAnalysis {
    pub total_duration: Duration,
    pub average_duration: Duration,
    pub slowest_operations: Vec<PerformanceMeasurement>,
    pub memory_usage_pattern: MemoryUsagePattern,
    pub cpu_usage_pattern: CpuUsagePattern,
    pub io_usage_pattern: IoUsagePattern,
    pub bottlenecks: Vec<Bottleneck>,
}

/// Memory usage pattern analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUsagePattern {
    pub peak_memory: Option<u64>,
    pub average_memory: Option<u64>,
    pub memory_growth_rate: Option<f64>, // bytes per second
    pub memory_leaks: Vec<String>,
}

/// CPU usage pattern analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuUsagePattern {
    pub peak_cpu: Option<f64>,
    pub average_cpu: Option<f64>,
    pub cpu_intensive_operations: Vec<String>,
}

/// I/O usage pattern analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoUsagePattern {
    pub total_io_operations: Option<u64>,
    pub io_intensive_operations: Vec<String>,
    pub io_bottlenecks: Vec<String>,
}

/// Identified performance bottleneck
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottleneck {
    pub operation: String,
    pub severity: BottleneckSeverity,
    pub description: String,
    pub suggestions: Vec<String>,
}

/// Bottleneck severity levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BottleneckSeverity {
    Low,
    Medium,
    High,
    Critical,
}

impl TestProfiler {
    /// Create a new test profiler
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            measurements: Vec::new(),
            current_measurement: None,
        }
    }

    /// Start profiling a specific operation
    pub fn start_operation(&mut self, name: String) {
        self.current_measurement = Some(name);
    }

    /// End profiling the current operation
    pub fn end_operation(&mut self) {
        if let Some(name) = self.current_measurement.take() {
            let duration = self.start_time.elapsed();
            let measurement = PerformanceMeasurement {
                name,
                duration,
                memory_before: None, // TODO: implement memory monitoring
                memory_after: None,  // TODO: implement memory monitoring
                memory_delta: None,  // TODO: implement memory monitoring
                cpu_usage: None,     // TODO: implement CPU monitoring
                io_operations: None, // TODO: implement I/O monitoring
                timestamp: chrono::Utc::now(),
            };
            self.measurements.push(measurement);
        }
    }

    /// Get all measurements
    pub fn get_measurements(&self) -> &[PerformanceMeasurement] {
        &self.measurements
    }

    /// Analyze performance and identify bottlenecks
    pub fn analyze_performance(&self) -> PerformanceAnalysis {
        let total_duration: Duration = self.measurements.iter().map(|m| m.duration).sum();

        let average_duration = if !self.measurements.is_empty() {
            Duration::from_nanos(total_duration.as_nanos() as u64 / self.measurements.len() as u64)
        } else {
            Duration::ZERO
        };

        // Find slowest operations (top 10%)
        let mut sorted_measurements = self.measurements.clone();
        sorted_measurements.sort_by(|a, b| b.duration.cmp(&a.duration));
        let slowest_count = (sorted_measurements.len() / 10).max(1);
        let slowest_operations = sorted_measurements
            .into_iter()
            .take(slowest_count)
            .collect();

        // Analyze memory usage
        let memory_pattern = self.analyze_memory_usage();

        // Analyze CPU usage
        let cpu_pattern = self.analyze_cpu_usage();

        // Analyze I/O usage
        let io_pattern = self.analyze_io_usage();

        // Identify bottlenecks
        let bottlenecks = self.identify_bottlenecks();

        PerformanceAnalysis {
            total_duration,
            average_duration,
            slowest_operations,
            memory_usage_pattern: memory_pattern,
            cpu_usage_pattern: cpu_pattern,
            io_usage_pattern: io_pattern,
            bottlenecks,
        }
    }

    /// Analyze memory usage patterns
    fn analyze_memory_usage(&self) -> MemoryUsagePattern {
        let memory_values: Vec<u64> = self
            .measurements
            .iter()
            .filter_map(|m| m.memory_after)
            .collect();

        let peak_memory = memory_values.iter().max().copied();
        let average_memory = if !memory_values.is_empty() {
            Some(memory_values.iter().sum::<u64>() / memory_values.len() as u64)
        } else {
            None
        };

        // Calculate memory growth rate
        let memory_growth_rate = if memory_values.len() > 1 {
            let first_memory = memory_values[0];
            let last_memory = memory_values[memory_values.len() - 1];
            let time_span = self
                .measurements
                .last()
                .unwrap()
                .timestamp
                .signed_duration_since(self.measurements.first().unwrap().timestamp)
                .num_seconds() as f64;

            if time_span > 0.0 {
                Some((last_memory as f64 - first_memory as f64) / time_span)
            } else {
                None
            }
        } else {
            None
        };

        // Detect potential memory leaks (simplified heuristic)
        let memory_leaks = self.detect_memory_leaks();

        MemoryUsagePattern {
            peak_memory,
            average_memory,
            memory_growth_rate,
            memory_leaks,
        }
    }

    /// Analyze CPU usage patterns
    fn analyze_cpu_usage(&self) -> CpuUsagePattern {
        let cpu_values: Vec<f64> = self
            .measurements
            .iter()
            .filter_map(|m| m.cpu_usage)
            .collect();

        let peak_cpu = cpu_values
            .iter()
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .copied();
        let average_cpu = if !cpu_values.is_empty() {
            Some(cpu_values.iter().sum::<f64>() / cpu_values.len() as f64)
        } else {
            None
        };

        // Find CPU-intensive operations
        let cpu_intensive_operations = self
            .measurements
            .iter()
            .filter(|m| m.cpu_usage.map_or(false, |cpu| cpu > 80.0))
            .map(|m| m.name.clone())
            .collect();

        CpuUsagePattern {
            peak_cpu,
            average_cpu,
            cpu_intensive_operations,
        }
    }

    /// Analyze I/O usage patterns
    fn analyze_io_usage(&self) -> IoUsagePattern {
        let total_io_operations = self
            .measurements
            .iter()
            .filter_map(|m| m.io_operations)
            .sum();

        let io_intensive_operations = self
            .measurements
            .iter()
            .filter(|m| m.io_operations.map_or(false, |io| io > 1000))
            .map(|m| m.name.clone())
            .collect();

        let io_bottlenecks = self
            .measurements
            .iter()
            .filter(|m| {
                m.io_operations.map_or(false, |io| io > 10000) && m.duration.as_millis() > 1000
            })
            .map(|m| m.name.clone())
            .collect();

        IoUsagePattern {
            total_io_operations: if total_io_operations > 0 {
                Some(total_io_operations)
            } else {
                None
            },
            io_intensive_operations,
            io_bottlenecks,
        }
    }

    /// Detect potential memory leaks
    fn detect_memory_leaks(&self) -> Vec<String> {
        let mut leaks = Vec::new();

        // Simple heuristic: if memory consistently grows without being freed
        let memory_values: Vec<u64> = self
            .measurements
            .iter()
            .filter_map(|m| m.memory_after)
            .collect();

        if memory_values.len() > 3 {
            let mut growing = true;
            for i in 1..memory_values.len() {
                if memory_values[i] <= memory_values[i - 1] {
                    growing = false;
                    break;
                }
            }

            if growing {
                leaks.push("Consistent memory growth detected".to_string());
            }
        }

        leaks
    }

    /// Identify performance bottlenecks
    fn identify_bottlenecks(&self) -> Vec<Bottleneck> {
        let mut bottlenecks = Vec::new();

        // Find operations that take too long
        for measurement in &self.measurements {
            if measurement.duration.as_secs() > 10 {
                bottlenecks.push(Bottleneck {
                    operation: measurement.name.clone(),
                    severity: BottleneckSeverity::Critical,
                    description: format!(
                        "Operation took {} seconds",
                        measurement.duration.as_secs()
                    ),
                    suggestions: vec![
                        "Consider optimizing the algorithm".to_string(),
                        "Check for unnecessary computations".to_string(),
                        "Profile the operation in detail".to_string(),
                    ],
                });
            } else if measurement.duration.as_millis() > 1000 {
                bottlenecks.push(Bottleneck {
                    operation: measurement.name.clone(),
                    severity: BottleneckSeverity::High,
                    description: format!(
                        "Operation took {} milliseconds",
                        measurement.duration.as_millis()
                    ),
                    suggestions: vec![
                        "Consider caching results".to_string(),
                        "Check for redundant operations".to_string(),
                    ],
                });
            }
        }

        // Find memory-intensive operations
        for measurement in &self.measurements {
            if let Some(memory) = measurement.memory_after {
                if memory > 100 * 1024 * 1024 {
                    // 100MB
                    bottlenecks.push(Bottleneck {
                        operation: measurement.name.clone(),
                        severity: BottleneckSeverity::High,
                        description: format!(
                            "Operation used {} MB of memory",
                            memory / 1024 / 1024
                        ),
                        suggestions: vec![
                            "Consider streaming data instead of loading all at once".to_string(),
                            "Check for memory leaks".to_string(),
                            "Optimize data structures".to_string(),
                        ],
                    });
                }
            }
        }

        // Find I/O bottlenecks
        for measurement in &self.measurements {
            if let Some(io_ops) = measurement.io_operations {
                if io_ops > 10000 && measurement.duration.as_millis() > 500 {
                    bottlenecks.push(Bottleneck {
                        operation: measurement.name.clone(),
                        severity: BottleneckSeverity::Medium,
                        description: format!(
                            "Operation performed {} I/O operations in {} ms",
                            io_ops,
                            measurement.duration.as_millis()
                        ),
                        suggestions: vec![
                            "Consider batching I/O operations".to_string(),
                            "Use async I/O if possible".to_string(),
                            "Cache frequently accessed data".to_string(),
                        ],
                    });
                }
            }
        }

        bottlenecks
    }

    /// Save profiler data to file
    pub fn save_data(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let data = serde_json::to_string_pretty(&self.measurements)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Load profiler data from file
    pub fn load_data(
        path: &PathBuf,
    ) -> Result<Vec<PerformanceMeasurement>, Box<dyn std::error::Error>> {
        let data = std::fs::read_to_string(path)?;
        let measurements: Vec<PerformanceMeasurement> = serde_json::from_str(&data)?;
        Ok(measurements)
    }
}

/// Test performance benchmark runner
pub struct PerformanceBenchmark {
    profiler: TestProfiler,
    iterations: u32,
}

impl PerformanceBenchmark {
    /// Create a new performance benchmark
    pub fn new(iterations: u32) -> Self {
        Self {
            profiler: TestProfiler::new(),
            iterations,
        }
    }

    /// Run a benchmark for a specific operation
    pub fn benchmark_operation<F>(&mut self, name: String, operation: F) -> Duration
    where
        F: Fn() -> Result<(), Box<dyn std::error::Error>>,
    {
        let mut total_duration = Duration::ZERO;

        for i in 0..self.iterations {
            self.profiler
                .start_operation(format!("{}_iteration_{}", name, i));
            let start = Instant::now();

            if let Err(e) = operation() {
                eprintln!("Benchmark operation failed: {}", e);
            }

            let duration = start.elapsed();
            total_duration += duration;
            self.profiler.end_operation();
        }

        Duration::from_nanos(total_duration.as_nanos() as u64 / self.iterations as u64)
    }

    /// Get profiler instance
    pub fn get_profiler(&self) -> &TestProfiler {
        &self.profiler
    }

    /// Generate benchmark report
    pub fn generate_report(&self) -> String {
        let analysis = self.profiler.analyze_performance();

        let mut report = String::new();
        report.push_str("# Performance Benchmark Report\n\n");
        report.push_str(&format!("Iterations: {}\n", self.iterations));
        report.push_str(&format!(
            "Total Duration: {:.2}s\n",
            analysis.total_duration.as_secs_f64()
        ));
        report.push_str(&format!(
            "Average Duration: {:.2}ms\n\n",
            analysis.average_duration.as_millis()
        ));

        if !analysis.slowest_operations.is_empty() {
            report.push_str("## Slowest Operations\n\n");
            for op in &analysis.slowest_operations {
                report.push_str(&format!(
                    "- {}: {:.2}ms\n",
                    op.name,
                    op.duration.as_millis()
                ));
            }
            report.push_str("\n");
        }

        if !analysis.bottlenecks.is_empty() {
            report.push_str("## Identified Bottlenecks\n\n");
            for bottleneck in &analysis.bottlenecks {
                report.push_str(&format!(
                    "### {} ({:?})\n",
                    bottleneck.operation, bottleneck.severity
                ));
                report.push_str(&format!("{}\n\n", bottleneck.description));
                report.push_str("Suggestions:\n");
                for suggestion in &bottleneck.suggestions {
                    report.push_str(&format!("- {}\n", suggestion));
                }
                report.push_str("\n");
            }
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_profiler_basic() {
        let mut profiler = TestProfiler::new();

        profiler.start_operation("test_operation".to_string());
        thread::sleep(Duration::from_millis(10));
        profiler.end_operation();

        let measurements = profiler.get_measurements();
        assert_eq!(measurements.len(), 1);
        assert_eq!(measurements[0].name, "test_operation");
        assert!(measurements[0].duration.as_millis() >= 10);
    }

    #[test]
    fn test_performance_analysis() {
        let mut profiler = TestProfiler::new();

        profiler.start_operation("fast_operation".to_string());
        thread::sleep(Duration::from_millis(1));
        profiler.end_operation();

        profiler.start_operation("slow_operation".to_string());
        thread::sleep(Duration::from_millis(100));
        profiler.end_operation();

        let analysis = profiler.analyze_performance();
        assert!(analysis.total_duration.as_millis() >= 101);
        assert_eq!(analysis.slowest_operations.len(), 1);
        assert_eq!(analysis.slowest_operations[0].name, "slow_operation");
    }

    #[test]
    fn test_benchmark_runner() {
        let mut benchmark = PerformanceBenchmark::new(3);

        let duration = benchmark.benchmark_operation("test_op".to_string(), || {
            thread::sleep(Duration::from_millis(1));
            Ok(())
        });

        assert!(duration.as_millis() >= 1);
        assert_eq!(benchmark.get_profiler().get_measurements().len(), 3);
    }
}
