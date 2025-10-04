//! Test debugging tools for JAC tests
//!
//! This module provides tools for debugging test failures, analyzing
//! test execution, and providing diagnostic information.

use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Test debugger for analyzing test failures and execution
pub struct TestDebugger {
    debug_info: HashMap<String, DebugInfo>,
    execution_log: Vec<ExecutionEvent>,
}

/// Debug information for a specific test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugInfo {
    pub test_name: String,
    pub status: TestStatus,
    pub execution_time: Duration,
    pub memory_usage: Option<u64>,
    pub error_message: Option<String>,
    pub stack_trace: Option<String>,
    pub input_data: Option<String>,
    pub expected_output: Option<String>,
    pub actual_output: Option<String>,
    pub environment_info: EnvironmentInfo,
    pub suggestions: Vec<String>,
}

/// Test execution status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestStatus {
    Passed,
    Failed,
    Ignored,
    Timeout,
    Panic,
}

/// Environment information for debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentInfo {
    pub rust_version: String,
    pub os: String,
    pub architecture: String,
    pub available_memory: Option<u64>,
    pub cpu_cores: Option<u32>,
    pub working_directory: String,
}

/// Execution event for logging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub test_name: String,
    pub event_type: EventType,
    pub message: String,
    pub duration: Option<Duration>,
}

/// Type of execution event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EventType {
    TestStart,
    TestEnd,
    Assertion,
    Error,
    Warning,
    Info,
}

impl TestDebugger {
    /// Create a new test debugger
    pub fn new() -> Self {
        Self {
            debug_info: HashMap::new(),
            execution_log: Vec::new(),
        }
    }

    /// Start debugging a test
    pub fn start_test(&mut self, test_name: String) {
        let debug_info = DebugInfo {
            test_name: test_name.clone(),
            status: TestStatus::Passed,
            execution_time: Duration::ZERO,
            memory_usage: None,
            error_message: None,
            stack_trace: None,
            input_data: None,
            expected_output: None,
            actual_output: None,
            environment_info: self.get_environment_info(),
            suggestions: Vec::new(),
        };

        self.debug_info.insert(test_name.clone(), debug_info);
        self.log_event(test_name, EventType::TestStart, "Test started".to_string());
    }

    /// End debugging a test
    pub fn end_test(&mut self, test_name: String, status: TestStatus, execution_time: Duration) {
        if let Some(debug_info) = self.debug_info.get_mut(&test_name) {
            debug_info.status = status.clone();
            debug_info.execution_time = execution_time;
        }

        self.log_event(test_name, EventType::TestEnd, format!("Test ended with status: {:?}", status));
    }

    /// Record a test failure
    pub fn record_failure(&mut self, test_name: String, error_message: String, stack_trace: Option<String>) {
        let suggestions = self.generate_suggestions(&error_message);

        if let Some(debug_info) = self.debug_info.get_mut(&test_name) {
            debug_info.status = TestStatus::Failed;
            debug_info.error_message = Some(error_message.clone());
            debug_info.stack_trace = stack_trace;
            debug_info.suggestions = suggestions;
        }

        self.log_event(test_name, EventType::Error, error_message);
    }

    /// Record test data for debugging
    pub fn record_test_data(&mut self, test_name: String, input_data: String, expected_output: String, actual_output: String) {
        if let Some(debug_info) = self.debug_info.get_mut(&test_name) {
            debug_info.input_data = Some(input_data);
            debug_info.expected_output = Some(expected_output);
            debug_info.actual_output = Some(actual_output);
        }
    }

    /// Log an execution event
    pub fn log_event(&mut self, test_name: String, event_type: EventType, message: String) {
        let event = ExecutionEvent {
            timestamp: chrono::Utc::now(),
            test_name,
            event_type,
            message,
            duration: None,
        };
        self.execution_log.push(event);
    }

    /// Get debug information for a test
    pub fn get_debug_info(&self, test_name: &str) -> Option<&DebugInfo> {
        self.debug_info.get(test_name)
    }

    /// Get all debug information
    pub fn get_all_debug_info(&self) -> &HashMap<String, DebugInfo> {
        &self.debug_info
    }

    /// Get execution log
    pub fn get_execution_log(&self) -> &[ExecutionEvent] {
        &self.execution_log
    }

    /// Generate debugging report
    pub fn generate_debug_report(&self) -> String {
        let mut report = String::new();
        report.push_str("# Test Debugging Report\n\n");
        report.push_str(&format!("Generated on: {}\n\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));

        // Summary
        let total_tests = self.debug_info.len();
        let failed_tests = self.debug_info.values().filter(|info| matches!(info.status, TestStatus::Failed)).count();
        let passed_tests = total_tests - failed_tests;

        report.push_str("## Summary\n\n");
        report.push_str(&format!("Total tests: {}\n", total_tests));
        report.push_str(&format!("Passed: {}\n", passed_tests));
        report.push_str(&format!("Failed: {}\n", failed_tests));
        report.push_str(&format!("Success rate: {:.1}%\n\n", (passed_tests as f64 / total_tests as f64) * 100.0));

        // Failed tests details
        if failed_tests > 0 {
            report.push_str("## Failed Tests\n\n");
            for (test_name, debug_info) in &self.debug_info {
                if matches!(debug_info.status, TestStatus::Failed) {
                    report.push_str(&format!("### {}\n", test_name));
                    report.push_str(&format!("**Status:** {:?}\n", debug_info.status));
                    report.push_str(&format!("**Execution time:** {:.2}ms\n", debug_info.execution_time.as_millis()));

                    if let Some(ref error) = debug_info.error_message {
                        report.push_str(&format!("**Error:** {}\n", error));
                    }

                    if let Some(ref stack_trace) = debug_info.stack_trace {
                        report.push_str(&format!("**Stack trace:**\n```\n{}\n```\n", stack_trace));
                    }

                    if let Some(ref input) = debug_info.input_data {
                        report.push_str(&format!("**Input data:**\n```\n{}\n```\n", input));
                    }

                    if let Some(ref expected) = debug_info.expected_output {
                        report.push_str(&format!("**Expected output:**\n```\n{}\n```\n", expected));
                    }

                    if let Some(ref actual) = debug_info.actual_output {
                        report.push_str(&format!("**Actual output:**\n```\n{}\n```\n", actual));
                    }

                    if !debug_info.suggestions.is_empty() {
                        report.push_str("**Suggestions:**\n");
                        for suggestion in &debug_info.suggestions {
                            report.push_str(&format!("- {}\n", suggestion));
                        }
                    }

                    report.push_str("\n");
                }
            }
        }

        // Environment information
        if let Some(first_info) = self.debug_info.values().next() {
            report.push_str("## Environment Information\n\n");
            report.push_str(&format!("Rust version: {}\n", first_info.environment_info.rust_version));
            report.push_str(&format!("OS: {}\n", first_info.environment_info.os));
            report.push_str(&format!("Architecture: {}\n", first_info.environment_info.architecture));
            report.push_str(&format!("Working directory: {}\n", first_info.environment_info.working_directory));
            if let Some(memory) = first_info.environment_info.available_memory {
                report.push_str(&format!("Available memory: {} MB\n", memory / 1024 / 1024));
            }
            if let Some(cores) = first_info.environment_info.cpu_cores {
                report.push_str(&format!("CPU cores: {}\n", cores));
            }
            report.push_str("\n");
        }

        // Execution timeline
        report.push_str("## Execution Timeline\n\n");
        for event in &self.execution_log {
            report.push_str(&format!("[{}] {} - {}: {}\n",
                event.timestamp.format("%H:%M:%S%.3f"),
                event.test_name,
                format!("{:?}", event.event_type),
                event.message
            ));
        }

        report
    }

    /// Save debug information to file
    pub fn save_debug_info(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let data = serde_json::to_string_pretty(&self.debug_info)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Load debug information from file
    pub fn load_debug_info(path: &PathBuf) -> Result<HashMap<String, DebugInfo>, Box<dyn std::error::Error>> {
        let data = std::fs::read_to_string(path)?;
        let debug_info: HashMap<String, DebugInfo> = serde_json::from_str(&data)?;
        Ok(debug_info)
    }

    /// Get environment information
    fn get_environment_info(&self) -> EnvironmentInfo {
        EnvironmentInfo {
            rust_version: std::env::var("RUSTC_SEMVER").unwrap_or_else(|_| "unknown".to_string()),
            os: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            available_memory: self.get_available_memory(),
            cpu_cores: Some(num_cpus::get() as u32),
            working_directory: std::env::current_dir().unwrap_or_default().to_string_lossy().to_string(),
        }
    }

    /// Get available memory (simplified)
    fn get_available_memory(&self) -> Option<u64> {
        // This is a simplified implementation
        // In a real implementation, you would use platform-specific APIs
        None
    }

    /// Generate suggestions based on error message
    fn generate_suggestions(&self, error_message: &str) -> Vec<String> {
        let mut suggestions = Vec::new();

        if error_message.contains("assertion") {
            suggestions.push("Check if the assertion condition is correct".to_string());
            suggestions.push("Verify input data matches expected format".to_string());
            suggestions.push("Add debug output to understand the failure".to_string());
        } else if error_message.contains("timeout") {
            suggestions.push("Check if the test is waiting for a condition that never occurs".to_string());
            suggestions.push("Consider increasing timeout limits if appropriate".to_string());
            suggestions.push("Look for potential deadlocks or infinite loops".to_string());
        } else if error_message.contains("memory") {
            suggestions.push("Check for memory leaks in the test".to_string());
            suggestions.push("Consider reducing test data size".to_string());
            suggestions.push("Verify that resources are properly cleaned up".to_string());
        } else if error_message.contains("io") {
            suggestions.push("Check file paths and permissions".to_string());
            suggestions.push("Verify that required files exist".to_string());
            suggestions.push("Check disk space availability".to_string());
        } else {
            suggestions.push("Review the error message for specific clues".to_string());
            suggestions.push("Check test setup and teardown".to_string());
            suggestions.push("Consider adding more detailed logging".to_string());
        }

        suggestions
    }
}

/// Test failure analyzer for detailed failure analysis
pub struct TestFailureAnalyzer {
    failures: Vec<DetailedFailure>,
}

/// Detailed failure information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetailedFailure {
    pub test_name: String,
    pub error_type: String,
    pub error_message: String,
    pub stack_trace: Option<String>,
    pub input_data: Option<String>,
    pub expected_output: Option<String>,
    pub actual_output: Option<String>,
    pub environment_context: EnvironmentInfo,
    pub root_cause: Option<String>,
    pub fix_suggestions: Vec<String>,
    pub similar_failures: Vec<String>,
}

impl TestFailureAnalyzer {
    /// Create a new failure analyzer
    pub fn new() -> Self {
        Self {
            failures: Vec::new(),
        }
    }

    /// Add a failure for analysis
    pub fn add_failure(&mut self, failure: DetailedFailure) {
        self.failures.push(failure);
    }

    /// Analyze failures and generate insights
    pub fn analyze_failures(&self) -> FailureAnalysisReport {
        let mut error_types = HashMap::new();
        let mut common_patterns = Vec::new();
        let mut root_causes = Vec::new();

        for failure in &self.failures {
            // Count error types
            *error_types.entry(failure.error_type.clone()).or_insert(0) += 1;

            // Analyze patterns
            if let Some(ref cause) = failure.root_cause {
                root_causes.push(cause.clone());
            }
        }

        // Find common patterns
        let mut pattern_counts = HashMap::new();
        for failure in &self.failures {
            if let Some(ref cause) = failure.root_cause {
                *pattern_counts.entry(cause.clone()).or_insert(0) += 1;
            }
        }

        for (pattern, count) in pattern_counts {
            if count > 1 {
                common_patterns.push((pattern, count));
            }
        }

        FailureAnalysisReport {
            total_failures: self.failures.len(),
            error_type_distribution: error_types,
            common_patterns,
            root_causes,
            recommendations: self.generate_recommendations(),
        }
    }

    /// Generate recommendations based on failure analysis
    fn generate_recommendations(&self) -> Vec<String> {
        let mut recommendations = Vec::new();

        if self.failures.is_empty() {
            return recommendations;
        }

        // Analyze error types
        let mut error_counts = HashMap::new();
        for failure in &self.failures {
            *error_counts.entry(&failure.error_type).or_insert(0) += 1;
        }

        let most_common_error = error_counts.iter()
            .max_by_key(|(_, count)| *count)
            .map(|(error_type, _)| error_type);

        if let Some(error_type) = most_common_error {
            match error_type.as_str() {
                "AssertionError" => {
                    recommendations.push("Review assertion logic and test data".to_string());
                    recommendations.push("Add more descriptive assertion messages".to_string());
                },
                "TimeoutError" => {
                    recommendations.push("Review timeout settings and test logic".to_string());
                    recommendations.push("Check for potential deadlocks".to_string());
                },
                "MemoryError" => {
                    recommendations.push("Implement memory monitoring in tests".to_string());
                    recommendations.push("Review test data size and cleanup".to_string());
                },
                _ => {
                    recommendations.push("Investigate the most common error type".to_string());
                }
            }
        }

        recommendations
    }
}

/// Failure analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureAnalysisReport {
    pub total_failures: usize,
    pub error_type_distribution: HashMap<String, usize>,
    pub common_patterns: Vec<(String, usize)>,
    pub root_causes: Vec<String>,
    pub recommendations: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_debugger_basic() {
        let mut debugger = TestDebugger::new();

        debugger.start_test("test_example".to_string());
        debugger.end_test("test_example".to_string(), TestStatus::Passed, Duration::from_millis(100));

        let debug_info = debugger.get_debug_info("test_example");
        assert!(debug_info.is_some());
        assert!(matches!(debug_info.unwrap().status, TestStatus::Passed));
    }

    #[test]
    fn test_failure_recording() {
        let mut debugger = TestDebugger::new();

        debugger.start_test("test_failing".to_string());
        debugger.record_failure(
            "test_failing".to_string(),
            "assertion failed".to_string(),
            Some("stack trace".to_string()),
        );
        debugger.end_test("test_failing".to_string(), TestStatus::Failed, Duration::from_millis(50));

        let debug_info = debugger.get_debug_info("test_failing");
        assert!(debug_info.is_some());
        assert!(matches!(debug_info.unwrap().status, TestStatus::Failed));
    }

    #[test]
    fn test_failure_analyzer() {
        let mut analyzer = TestFailureAnalyzer::new();

        let failure = DetailedFailure {
            test_name: "test_failing".to_string(),
            error_type: "AssertionError".to_string(),
            error_message: "assertion failed".to_string(),
            stack_trace: Some("stack trace".to_string()),
            input_data: None,
            expected_output: None,
            actual_output: None,
            environment_context: EnvironmentInfo {
                rust_version: "1.0.0".to_string(),
                os: "linux".to_string(),
                architecture: "x86_64".to_string(),
                available_memory: None,
                cpu_cores: Some(4),
                working_directory: "/tmp".to_string(),
            },
            root_cause: Some("incorrect assertion".to_string()),
            fix_suggestions: vec!["fix assertion".to_string()],
            similar_failures: vec![],
        };

        analyzer.add_failure(failure);
        let report = analyzer.analyze_failures();

        assert_eq!(report.total_failures, 1);
        assert!(report.error_type_distribution.contains_key("AssertionError"));
    }
}
