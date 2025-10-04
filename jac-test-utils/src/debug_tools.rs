//! Debugging and performance visualization tools for JAC tests
//!
//! This module provides tools for analyzing test failures, monitoring performance,
//! and visualizing test results to improve debugging and maintenance.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// Test execution metrics for performance monitoring
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestMetrics {
    pub test_name: String,
    pub execution_time: Duration,
    pub memory_usage_bytes: Option<u64>,
    pub cpu_usage_percent: Option<f64>,
    pub io_operations: Option<u64>,
    pub assertions_count: u32,
    pub success: bool,
    pub error_message: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Test performance summary for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSummary {
    pub total_tests: u32,
    pub passed_tests: u32,
    pub failed_tests: u32,
    pub total_execution_time: Duration,
    pub average_execution_time: Duration,
    pub slowest_test: Option<String>,
    pub fastest_test: Option<String>,
    pub memory_peak_bytes: Option<u64>,
    pub cpu_peak_percent: Option<f64>,
}

/// Test failure analysis for debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureAnalysis {
    pub test_name: String,
    pub error_type: String,
    pub error_message: String,
    pub stack_trace: Option<String>,
    pub input_data: Option<String>,
    pub expected_output: Option<String>,
    pub actual_output: Option<String>,
    pub suggestions: Vec<String>,
}

/// Test result visualization data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestVisualization {
    pub performance_chart: PerformanceChart,
    pub failure_breakdown: FailureBreakdown,
    pub memory_usage: MemoryUsageChart,
    pub execution_timeline: ExecutionTimeline,
}

/// Performance chart data for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceChart {
    pub test_names: Vec<String>,
    pub execution_times: Vec<f64>, // in seconds
    pub memory_usage: Vec<Option<f64>>, // in MB
    pub cpu_usage: Vec<Option<f64>>, // in percent
}

/// Failure breakdown for visualization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureBreakdown {
    pub error_types: HashMap<String, u32>,
    pub failure_rate: f64,
    pub most_common_error: Option<String>,
    pub failure_trend: Vec<f64>, // failure rate over time
}

/// Memory usage chart data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryUsageChart {
    pub test_names: Vec<String>,
    pub peak_memory: Vec<Option<f64>>, // in MB
    pub average_memory: Vec<Option<f64>>, // in MB
    pub memory_trend: Vec<f64>, // memory usage over time
}

/// Execution timeline data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTimeline {
    pub timestamps: Vec<chrono::DateTime<chrono::Utc>>,
    pub test_names: Vec<String>,
    pub durations: Vec<f64>, // in seconds
    pub status: Vec<String>, // "passed", "failed", "ignored"
}

/// Test performance monitor for tracking metrics during test execution
pub struct TestPerformanceMonitor {
    start_time: Instant,
    metrics: Vec<TestMetrics>,
    current_test: Option<String>,
}

impl TestPerformanceMonitor {
    /// Create a new performance monitor
    pub fn new() -> Self {
        Self {
            start_time: Instant::now(),
            metrics: Vec::new(),
            current_test: None,
        }
    }

    /// Start monitoring a specific test
    pub fn start_test(&mut self, test_name: String) {
        self.current_test = Some(test_name);
    }

    /// End monitoring the current test and record metrics
    pub fn end_test(&mut self, success: bool, error_message: Option<String>) {
        if let Some(test_name) = self.current_test.take() {
            let execution_time = self.start_time.elapsed();
            let metrics = TestMetrics {
                test_name,
                execution_time,
                memory_usage_bytes: None, // TODO: implement memory monitoring
                cpu_usage_percent: None,  // TODO: implement CPU monitoring
                io_operations: None,      // TODO: implement I/O monitoring
                assertions_count: 0,      // TODO: implement assertion counting
                success,
                error_message,
                timestamp: chrono::Utc::now(),
            };
            self.metrics.push(metrics);
        }
    }

    /// Get all recorded metrics
    pub fn get_metrics(&self) -> &[TestMetrics] {
        &self.metrics
    }

    /// Generate performance summary
    pub fn generate_summary(&self) -> PerformanceSummary {
        let total_tests = self.metrics.len() as u32;
        let passed_tests = self.metrics.iter().filter(|m| m.success).count() as u32;
        let failed_tests = total_tests - passed_tests;

        let total_execution_time: Duration = self.metrics.iter()
            .map(|m| m.execution_time)
            .sum();

        let average_execution_time = if total_tests > 0 {
            Duration::from_nanos(total_execution_time.as_nanos() as u64 / total_tests as u64)
        } else {
            Duration::ZERO
        };

        let slowest_test = self.metrics.iter()
            .max_by_key(|m| m.execution_time)
            .map(|m| m.test_name.clone());

        let fastest_test = self.metrics.iter()
            .min_by_key(|m| m.execution_time)
            .map(|m| m.test_name.clone());

        let memory_peak_bytes = self.metrics.iter()
            .filter_map(|m| m.memory_usage_bytes)
            .max();

        let cpu_peak_percent = self.metrics.iter()
            .filter_map(|m| m.cpu_usage_percent)
            .max_by(|a, b| a.partial_cmp(b).unwrap());

        PerformanceSummary {
            total_tests,
            passed_tests,
            failed_tests,
            total_execution_time,
            average_execution_time,
            slowest_test,
            fastest_test,
            memory_peak_bytes,
            cpu_peak_percent,
        }
    }

    /// Generate test visualization data
    pub fn generate_visualization(&self) -> TestVisualization {
        let test_names: Vec<String> = self.metrics.iter()
            .map(|m| m.test_name.clone())
            .collect();

        let execution_times: Vec<f64> = self.metrics.iter()
            .map(|m| m.execution_time.as_secs_f64())
            .collect();

        let memory_usage: Vec<Option<f64>> = self.metrics.iter()
            .map(|m| m.memory_usage_bytes.map(|b| b as f64 / 1_048_576.0)) // Convert to MB
            .collect();

        let cpu_usage: Vec<Option<f64>> = self.metrics.iter()
            .map(|m| m.cpu_usage_percent)
            .collect();

        let performance_chart = PerformanceChart {
            test_names: test_names.clone(),
            execution_times,
            memory_usage: memory_usage.clone(),
            cpu_usage,
        };

        // Analyze failures
        let mut error_types = HashMap::new();
        for metric in &self.metrics {
            if !metric.success {
                if let Some(ref error) = metric.error_message {
                    let error_type = extract_error_type(error);
                    *error_types.entry(error_type).or_insert(0) += 1;
                }
            }
        }

        let failure_rate = if self.metrics.is_empty() {
            0.0
        } else {
            self.metrics.iter().filter(|m| !m.success).count() as f64 / self.metrics.len() as f64
        };

        let most_common_error = error_types.iter()
            .max_by_key(|(_, count)| *count)
            .map(|(error_type, _)| error_type.clone());

        let failure_breakdown = FailureBreakdown {
            error_types,
            failure_rate,
            most_common_error,
            failure_trend: vec![failure_rate], // TODO: implement trend analysis
        };

        let memory_usage_chart = MemoryUsageChart {
            test_names: test_names.clone(),
            peak_memory: memory_usage.clone(),
            average_memory: memory_usage.clone(),
            memory_trend: vec![0.0], // TODO: implement trend analysis
        };

        let execution_timeline = ExecutionTimeline {
            timestamps: self.metrics.iter().map(|m| m.timestamp).collect(),
            test_names,
            durations: self.metrics.iter().map(|m| m.execution_time.as_secs_f64()).collect(),
            status: self.metrics.iter().map(|m| if m.success { "passed".to_string() } else { "failed".to_string() }).collect(),
        };

        TestVisualization {
            performance_chart,
            failure_breakdown,
            memory_usage: memory_usage_chart,
            execution_timeline,
        }
    }

    /// Save metrics to a file
    pub fn save_metrics(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(&self.metrics)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load metrics from a file
    pub fn load_metrics(path: &PathBuf) -> Result<Vec<TestMetrics>, Box<dyn std::error::Error>> {
        let json = std::fs::read_to_string(path)?;
        let metrics: Vec<TestMetrics> = serde_json::from_str(&json)?;
        Ok(metrics)
    }
}

/// Extract error type from error message for categorization
fn extract_error_type(error_message: &str) -> String {
    // Simple error type extraction - can be enhanced
    if error_message.contains("assertion") {
        "AssertionError".to_string()
    } else if error_message.contains("timeout") {
        "TimeoutError".to_string()
    } else if error_message.contains("memory") {
        "MemoryError".to_string()
    } else if error_message.contains("io") {
        "IoError".to_string()
    } else {
        "UnknownError".to_string()
    }
}

/// Test failure analyzer for debugging
pub struct TestFailureAnalyzer {
    failures: Vec<FailureAnalysis>,
}

impl TestFailureAnalyzer {
    /// Create a new failure analyzer
    pub fn new() -> Self {
        Self {
            failures: Vec::new(),
        }
    }

    /// Add a failure for analysis
    pub fn add_failure(&mut self, test_name: String, error_message: String, stack_trace: Option<String>) {
        let error_type = extract_error_type(&error_message);
        let suggestions = generate_suggestions(&error_type, &error_message);

        let analysis = FailureAnalysis {
            test_name,
            error_type,
            error_message,
            stack_trace,
            input_data: None, // TODO: implement input data capture
            expected_output: None, // TODO: implement expected output capture
            actual_output: None, // TODO: implement actual output capture
            suggestions,
        };

        self.failures.push(analysis);
    }

    /// Generate failure report
    pub fn generate_report(&self) -> String {
        let mut report = String::new();
        report.push_str("# Test Failure Analysis Report\n\n");

        if self.failures.is_empty() {
            report.push_str("No failures to analyze.\n");
            return report;
        }

        // Group failures by error type
        let mut failures_by_type: HashMap<String, Vec<&FailureAnalysis>> = HashMap::new();
        for failure in &self.failures {
            failures_by_type.entry(failure.error_type.clone()).or_default().push(failure);
        }

        // Summary
        report.push_str(&format!("Total failures: {}\n", self.failures.len()));
        report.push_str(&format!("Error types: {}\n\n", failures_by_type.len()));

        // Detailed analysis for each error type
        for (error_type, failures) in failures_by_type {
            report.push_str(&format!("## {}\n\n", error_type));
            report.push_str(&format!("Count: {}\n\n", failures.len()));

            for failure in failures {
                report.push_str(&format!("### {}\n", failure.test_name));
                report.push_str(&format!("Error: {}\n", failure.error_message));

                if !failure.suggestions.is_empty() {
                    report.push_str("Suggestions:\n");
                    for suggestion in &failure.suggestions {
                        report.push_str(&format!("- {}\n", suggestion));
                    }
                }

                if let Some(ref stack_trace) = failure.stack_trace {
                    report.push_str(&format!("Stack trace:\n```\n{}\n```\n", stack_trace));
                }

                report.push_str("\n");
            }
        }

        report
    }

    /// Save failure analysis to file
    pub fn save_report(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let report = self.generate_report();
        std::fs::write(path, report)?;
        Ok(())
    }
}

/// Generate suggestions based on error type and message
fn generate_suggestions(error_type: &str, _error_message: &str) -> Vec<String> {
    let mut suggestions = Vec::new();

    match error_type {
        "AssertionError" => {
            suggestions.push("Check if the assertion condition is correct".to_string());
            suggestions.push("Verify input data matches expected format".to_string());
            suggestions.push("Consider adding debug output to understand the failure".to_string());
        },
        "TimeoutError" => {
            suggestions.push("Check if the test is waiting for a condition that never occurs".to_string());
            suggestions.push("Consider increasing timeout limits if appropriate".to_string());
            suggestions.push("Look for potential deadlocks or infinite loops".to_string());
        },
        "MemoryError" => {
            suggestions.push("Check for memory leaks in the test".to_string());
            suggestions.push("Consider reducing test data size".to_string());
            suggestions.push("Verify that resources are properly cleaned up".to_string());
        },
        "IoError" => {
            suggestions.push("Check file paths and permissions".to_string());
            suggestions.push("Verify that required files exist".to_string());
            suggestions.push("Check disk space availability".to_string());
        },
        _ => {
            suggestions.push("Review the error message for specific clues".to_string());
            suggestions.push("Check test setup and teardown".to_string());
            suggestions.push("Consider adding more detailed logging".to_string());
        }
    }

    suggestions
}

/// Test maintenance tools for managing test data and fixtures
pub struct TestMaintenanceTools {
    test_data_dir: PathBuf,
}

impl TestMaintenanceTools {
    /// Create new maintenance tools
    pub fn new(test_data_dir: PathBuf) -> Self {
        Self { test_data_dir }
    }

    /// Validate test data integrity
    pub fn validate_test_data(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut issues = Vec::new();

        if !self.test_data_dir.exists() {
            issues.push("Test data directory does not exist".to_string());
            return Ok(issues);
        }

        // Check for common test data issues
        let entries = std::fs::read_dir(&self.test_data_dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                // Check file size
                let metadata = std::fs::metadata(&path)?;
                if metadata.len() == 0 {
                    issues.push(format!("Empty file: {}", path.display()));
                }

                // Check file extension
                if let Some(ext) = path.extension() {
                    if ext == "json" || ext == "ndjson" {
                        // Validate JSON format
                        if let Err(e) = self.validate_json_file(&path) {
                            issues.push(format!("Invalid JSON in {}: {}", path.display(), e));
                        }
                    }
                }
            }
        }

        Ok(issues)
    }

    /// Validate JSON file format
    fn validate_json_file(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;

        // Try to parse as JSON
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            serde_json::from_str::<serde_json::Value>(&content)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("ndjson") {
            // Parse as NDJSON (one JSON object per line)
            for line in content.lines() {
                if !line.trim().is_empty() {
                    serde_json::from_str::<serde_json::Value>(line)?;
                }
            }
        }

        Ok(())
    }

    /// Generate test data report
    pub fn generate_test_data_report(&self) -> Result<String, Box<dyn std::error::Error>> {
        let mut report = String::new();
        report.push_str("# Test Data Report\n\n");

        if !self.test_data_dir.exists() {
            report.push_str("Test data directory does not exist.\n");
            return Ok(report);
        }

        let mut total_files = 0;
        let mut total_size = 0;
        let mut json_files = 0;
        let mut ndjson_files = 0;
        let mut other_files = 0;

        let entries = std::fs::read_dir(&self.test_data_dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                total_files += 1;
                let metadata = std::fs::metadata(&path)?;
                total_size += metadata.len();

                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    match ext {
                        "json" => json_files += 1,
                        "ndjson" => ndjson_files += 1,
                        _ => other_files += 1,
                    }
                } else {
                    other_files += 1;
                }
            }
        }

        report.push_str(&format!("Total files: {}\n", total_files));
        report.push_str(&format!("Total size: {} bytes ({:.2} MB)\n", total_size, total_size as f64 / 1_048_576.0));
        report.push_str(&format!("JSON files: {}\n", json_files));
        report.push_str(&format!("NDJSON files: {}\n", ndjson_files));
        report.push_str(&format!("Other files: {}\n", other_files));

        Ok(report)
    }

    /// Clean up old test artifacts
    pub fn cleanup_artifacts(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let mut cleaned = Vec::new();

        // Look for common test artifact patterns
        let patterns = vec!["*.tmp", "*.temp", "test_output_*", "debug_*"];

        for pattern in patterns {
            let glob_pattern = self.test_data_dir.join(pattern);
            if let Ok(entries) = glob::glob(glob_pattern.to_str().unwrap()) {
                for entry in entries {
                    if let Ok(path) = entry {
                        if path.is_file() {
                            std::fs::remove_file(&path)?;
                            cleaned.push(path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }

        Ok(cleaned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_performance_monitor() {
        let mut monitor = TestPerformanceMonitor::new();

        monitor.start_test("test_example".to_string());
        std::thread::sleep(Duration::from_millis(10));
        monitor.end_test(true, None);

        let metrics = monitor.get_metrics();
        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].test_name, "test_example");
        assert!(metrics[0].success);
    }

    #[test]
    fn test_failure_analyzer() {
        let mut analyzer = TestFailureAnalyzer::new();

        analyzer.add_failure(
            "test_failing".to_string(),
            "assertion failed: expected 5, got 3".to_string(),
            Some("stack trace here".to_string()),
        );

        let report = analyzer.generate_report();
        assert!(report.contains("Test Failure Analysis Report"));
        assert!(report.contains("test_failing"));
        assert!(report.contains("AssertionError"));
    }

    #[test]
    fn test_error_type_extraction() {
        assert_eq!(extract_error_type("assertion failed"), "AssertionError");
        assert_eq!(extract_error_type("timeout occurred"), "TimeoutError");
        assert_eq!(extract_error_type("memory allocation failed"), "MemoryError");
        assert_eq!(extract_error_type("io error"), "IoError");
        assert_eq!(extract_error_type("unknown error"), "UnknownError");
    }
}
