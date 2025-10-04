//! Test result visualization tools for JAC tests
//!
//! This module provides tools for generating visual reports and charts
//! to help understand test results and performance.

use crate::debug_tools::{TestMetrics, TestVisualization, PerformanceSummary};
use std::time::Duration;
use std::path::PathBuf;
use std::collections::HashMap;

/// HTML report generator for test results
pub struct HtmlReportGenerator {
    template_dir: Option<PathBuf>,
}

impl HtmlReportGenerator {
    /// Create a new HTML report generator
    pub fn new() -> Self {
        Self { template_dir: None }
    }

    /// Set custom template directory
    pub fn with_template_dir(mut self, dir: PathBuf) -> Self {
        self.template_dir = Some(dir);
        self
    }

    /// Generate HTML report from test metrics
    pub fn generate_report(&self, metrics: &[TestMetrics], output_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let summary = self.calculate_summary(metrics);
        let visualization = self.generate_visualization_data(metrics);

        let html = self.render_html(&summary, &visualization)?;
        std::fs::write(output_path, html)?;

        Ok(())
    }

    /// Calculate performance summary from metrics
    fn calculate_summary(&self, metrics: &[TestMetrics]) -> PerformanceSummary {
        let total_tests = metrics.len() as u32;
        let passed_tests = metrics.iter().filter(|m| m.success).count() as u32;
        let failed_tests = total_tests - passed_tests;

        let total_execution_time: Duration = metrics.iter()
            .map(|m| m.execution_time)
            .sum();

        let average_execution_time = if total_tests > 0 {
            std::time::Duration::from_nanos(total_execution_time.as_nanos() as u64 / total_tests as u64)
        } else {
            std::time::Duration::ZERO
        };

        let slowest_test = metrics.iter()
            .max_by_key(|m| m.execution_time)
            .map(|m| m.test_name.clone());

        let fastest_test = metrics.iter()
            .min_by_key(|m| m.execution_time)
            .map(|m| m.test_name.clone());

        let memory_peak_bytes = metrics.iter()
            .filter_map(|m| m.memory_usage_bytes)
            .max();

        let cpu_peak_percent = metrics.iter()
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

    /// Generate visualization data from metrics
    fn generate_visualization_data(&self, metrics: &[TestMetrics]) -> TestVisualization {
        let test_names: Vec<String> = metrics.iter()
            .map(|m| m.test_name.clone())
            .collect();

        let execution_times: Vec<f64> = metrics.iter()
            .map(|m| m.execution_time.as_secs_f64())
            .collect();

        let memory_usage: Vec<Option<f64>> = metrics.iter()
            .map(|m| m.memory_usage_bytes.map(|b| b as f64 / 1_048_576.0)) // Convert to MB
            .collect();

        let cpu_usage: Vec<Option<f64>> = metrics.iter()
            .map(|m| m.cpu_usage_percent)
            .collect();

        let performance_chart = crate::debug_tools::PerformanceChart {
            test_names: test_names.clone(),
            execution_times,
            memory_usage: memory_usage.clone(),
            cpu_usage,
        };

        // Analyze failures
        let mut error_types = HashMap::new();
        for metric in metrics {
            if !metric.success {
                if let Some(ref error) = metric.error_message {
                    let error_type = self.extract_error_type(error);
                    *error_types.entry(error_type).or_insert(0) += 1;
                }
            }
        }

        let failure_rate = if metrics.is_empty() {
            0.0
        } else {
            metrics.iter().filter(|m| !m.success).count() as f64 / metrics.len() as f64
        };

        let most_common_error = error_types.iter()
            .max_by_key(|(_, count)| *count)
            .map(|(error_type, _)| error_type.clone());

        let failure_breakdown = crate::debug_tools::FailureBreakdown {
            error_types,
            failure_rate,
            most_common_error,
            failure_trend: vec![failure_rate], // TODO: implement trend analysis
        };

        let memory_usage_chart = crate::debug_tools::MemoryUsageChart {
            test_names: test_names.clone(),
            peak_memory: memory_usage.clone(),
            average_memory: memory_usage.clone(),
            memory_trend: vec![0.0], // TODO: implement trend analysis
        };

        let execution_timeline = crate::debug_tools::ExecutionTimeline {
            timestamps: metrics.iter().map(|m| m.timestamp).collect(),
            test_names,
            durations: metrics.iter().map(|m| m.execution_time.as_secs_f64()).collect(),
            status: metrics.iter().map(|m| if m.success { "passed".to_string() } else { "failed".to_string() }).collect(),
        };

        TestVisualization {
            performance_chart,
            failure_breakdown,
            memory_usage: memory_usage_chart,
            execution_timeline,
        }
    }

    /// Extract error type from error message
    fn extract_error_type(&self, error_message: &str) -> String {
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

    /// Render HTML report
    fn render_html(&self, summary: &PerformanceSummary, visualization: &TestVisualization) -> Result<String, Box<dyn std::error::Error>> {
        let mut html = String::new();

        html.push_str("<!DOCTYPE html>\n");
        html.push_str("<html lang=\"en\">\n");
        html.push_str("<head>\n");
        html.push_str("    <meta charset=\"UTF-8\">\n");
        html.push_str("    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n");
        html.push_str("    <title>JAC Test Results Report</title>\n");
        html.push_str("    <script src=\"https://cdn.jsdelivr.net/npm/chart.js\"></script>\n");
        html.push_str("    <style>\n");
        html.push_str(self.get_css_styles());
        html.push_str("    </style>\n");
        html.push_str("</head>\n");
        html.push_str("<body>\n");

        // Header
        html.push_str("    <header>\n");
        html.push_str("        <h1>JAC Test Results Report</h1>\n");
        html.push_str(&format!("        <p>Generated on: {}</p>\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
        html.push_str("    </header>\n");

        // Summary section
        html.push_str("    <section class=\"summary\">\n");
        html.push_str("        <h2>Test Summary</h2>\n");
        html.push_str("        <div class=\"summary-grid\">\n");
        html.push_str(&format!("            <div class=\"summary-item\">\n"));
        html.push_str(&format!("                <h3>Total Tests</h3>\n"));
        html.push_str(&format!("                <span class=\"number\">{}</span>\n", summary.total_tests));
        html.push_str("            </div>\n");
        html.push_str(&format!("            <div class=\"summary-item\">\n"));
        html.push_str(&format!("                <h3>Passed</h3>\n"));
        html.push_str(&format!("                <span class=\"number success\">{}</span>\n", summary.passed_tests));
        html.push_str("            </div>\n");
        html.push_str(&format!("            <div class=\"summary-item\">\n"));
        html.push_str(&format!("                <h3>Failed</h3>\n"));
        html.push_str(&format!("                <span class=\"number error\">{}</span>\n", summary.failed_tests));
        html.push_str("            </div>\n");
        html.push_str(&format!("            <div class=\"summary-item\">\n"));
        html.push_str(&format!("                <h3>Success Rate</h3>\n"));
        let success_rate = if summary.total_tests > 0 {
            (summary.passed_tests as f64 / summary.total_tests as f64) * 100.0
        } else {
            0.0
        };
        html.push_str(&format!("                <span class=\"number\">{:.1}%</span>\n", success_rate));
        html.push_str("            </div>\n");
        html.push_str("        </div>\n");
        html.push_str("    </section>\n");

        // Performance section
        html.push_str("    <section class=\"performance\">\n");
        html.push_str("        <h2>Performance Metrics</h2>\n");
        html.push_str("        <div class=\"performance-grid\">\n");
        html.push_str(&format!("            <div class=\"performance-item\">\n"));
        html.push_str(&format!("                <h3>Total Execution Time</h3>\n"));
        html.push_str(&format!("                <span class=\"number\">{:.2}s</span>\n", summary.total_execution_time.as_secs_f64()));
        html.push_str("            </div>\n");
        html.push_str(&format!("            <div class=\"performance-item\">\n"));
        html.push_str(&format!("                <h3>Average Test Time</h3>\n"));
        html.push_str(&format!("                <span class=\"number\">{:.2}ms</span>\n", summary.average_execution_time.as_millis()));
        html.push_str("            </div>\n");
        if let Some(ref slowest) = summary.slowest_test {
            html.push_str(&format!("            <div class=\"performance-item\">\n"));
            html.push_str(&format!("                <h3>Slowest Test</h3>\n"));
            html.push_str(&format!("                <span class=\"text\">{}</span>\n", slowest));
            html.push_str("            </div>\n");
        }
        if let Some(ref fastest) = summary.fastest_test {
            html.push_str(&format!("            <div class=\"performance-item\">\n"));
            html.push_str(&format!("                <h3>Fastest Test</h3>\n"));
            html.push_str(&format!("                <span class=\"text\">{}</span>\n", fastest));
            html.push_str("            </div>\n");
        }
        html.push_str("        </div>\n");
        html.push_str("    </section>\n");

        // Charts section
        html.push_str("    <section class=\"charts\">\n");
        html.push_str("        <h2>Performance Charts</h2>\n");
        html.push_str("        <div class=\"chart-container\">\n");
        html.push_str("            <canvas id=\"executionTimeChart\"></canvas>\n");
        html.push_str("        </div>\n");
        html.push_str("        <div class=\"chart-container\">\n");
        html.push_str("            <canvas id=\"memoryUsageChart\"></canvas>\n");
        html.push_str("        </div>\n");
        html.push_str("    </section>\n");

        // JavaScript for charts
        html.push_str("    <script>\n");
        html.push_str(&self.generate_chart_js(visualization));
        html.push_str("    </script>\n");

        html.push_str("</body>\n");
        html.push_str("</html>\n");

        Ok(html)
    }

    /// Get CSS styles for the HTML report
    fn get_css_styles(&self) -> &'static str {
        r#"
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            line-height: 1.6;
            color: #333;
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
            background-color: #f5f5f5;
        }

        header {
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            border-radius: 10px;
            margin-bottom: 30px;
            text-align: center;
        }

        header h1 {
            margin: 0 0 10px 0;
            font-size: 2.5em;
        }

        .summary {
            background: white;
            padding: 30px;
            border-radius: 10px;
            margin-bottom: 30px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }

        .summary-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 20px;
            margin-top: 20px;
        }

        .summary-item {
            text-align: center;
            padding: 20px;
            background: #f8f9fa;
            border-radius: 8px;
        }

        .summary-item h3 {
            margin: 0 0 10px 0;
            color: #666;
            font-size: 0.9em;
            text-transform: uppercase;
            letter-spacing: 1px;
        }

        .number {
            font-size: 2em;
            font-weight: bold;
            color: #333;
        }

        .number.success {
            color: #28a745;
        }

        .number.error {
            color: #dc3545;
        }

        .performance {
            background: white;
            padding: 30px;
            border-radius: 10px;
            margin-bottom: 30px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }

        .performance-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
            gap: 20px;
            margin-top: 20px;
        }

        .performance-item {
            padding: 20px;
            background: #f8f9fa;
            border-radius: 8px;
        }

        .performance-item h3 {
            margin: 0 0 10px 0;
            color: #666;
            font-size: 0.9em;
            text-transform: uppercase;
            letter-spacing: 1px;
        }

        .text {
            font-size: 1.1em;
            color: #333;
        }

        .charts {
            background: white;
            padding: 30px;
            border-radius: 10px;
            margin-bottom: 30px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }

        .chart-container {
            margin: 20px 0;
            height: 400px;
        }

        canvas {
            max-width: 100%;
            height: auto;
        }

        h2 {
            color: #333;
            border-bottom: 2px solid #667eea;
            padding-bottom: 10px;
            margin-bottom: 20px;
        }
        "#
    }

    /// Generate JavaScript for Chart.js charts
    fn generate_chart_js(&self, visualization: &TestVisualization) -> String {
        let mut js = String::new();

        // Execution time chart
        js.push_str("const executionTimeCtx = document.getElementById('executionTimeChart').getContext('2d');\n");
        js.push_str("new Chart(executionTimeCtx, {\n");
        js.push_str("    type: 'bar',\n");
        js.push_str("    data: {\n");
        js.push_str("        labels: [");
        for (i, name) in visualization.performance_chart.test_names.iter().enumerate() {
            if i > 0 { js.push_str(", "); }
            js.push_str(&format!("\"{}\"", name));
        }
        js.push_str("],\n");
        js.push_str("        datasets: [{\n");
        js.push_str("            label: 'Execution Time (seconds)',\n");
        js.push_str("            data: [");
        for (i, time) in visualization.performance_chart.execution_times.iter().enumerate() {
            if i > 0 { js.push_str(", "); }
            js.push_str(&format!("{:.3}", time));
        }
        js.push_str("],\n");
        js.push_str("            backgroundColor: 'rgba(102, 126, 234, 0.8)',\n");
        js.push_str("            borderColor: 'rgba(102, 126, 234, 1)',\n");
        js.push_str("            borderWidth: 1\n");
        js.push_str("        }]\n");
        js.push_str("    },\n");
        js.push_str("    options: {\n");
        js.push_str("        responsive: true,\n");
        js.push_str("        maintainAspectRatio: false,\n");
        js.push_str("        scales: {\n");
        js.push_str("            y: {\n");
        js.push_str("                beginAtZero: true,\n");
        js.push_str("                title: {\n");
        js.push_str("                    display: true,\n");
        js.push_str("                    text: 'Time (seconds)'\n");
        js.push_str("                }\n");
        js.push_str("            }\n");
        js.push_str("        }\n");
        js.push_str("    }\n");
        js.push_str("});\n\n");

        // Memory usage chart
        js.push_str("const memoryUsageCtx = document.getElementById('memoryUsageChart').getContext('2d');\n");
        js.push_str("new Chart(memoryUsageCtx, {\n");
        js.push_str("    type: 'line',\n");
        js.push_str("    data: {\n");
        js.push_str("        labels: [");
        for (i, name) in visualization.performance_chart.test_names.iter().enumerate() {
            if i > 0 { js.push_str(", "); }
            js.push_str(&format!("\"{}\"", name));
        }
        js.push_str("],\n");
        js.push_str("        datasets: [{\n");
        js.push_str("            label: 'Memory Usage (MB)',\n");
        js.push_str("            data: [");
        for (i, memory) in visualization.performance_chart.memory_usage.iter().enumerate() {
            if i > 0 { js.push_str(", "); }
            match memory {
                Some(m) => js.push_str(&format!("{:.2}", m)),
                None => js.push_str("null"),
            }
        }
        js.push_str("],\n");
        js.push_str("            backgroundColor: 'rgba(118, 75, 162, 0.2)',\n");
        js.push_str("            borderColor: 'rgba(118, 75, 162, 1)',\n");
        js.push_str("            borderWidth: 2,\n");
        js.push_str("            fill: true\n");
        js.push_str("        }]\n");
        js.push_str("    },\n");
        js.push_str("    options: {\n");
        js.push_str("        responsive: true,\n");
        js.push_str("        maintainAspectRatio: false,\n");
        js.push_str("        scales: {\n");
        js.push_str("            y: {\n");
        js.push_str("                beginAtZero: true,\n");
        js.push_str("                title: {\n");
        js.push_str("                    display: true,\n");
        js.push_str("                    text: 'Memory (MB)'\n");
        js.push_str("                }\n");
        js.push_str("        }\n");
        js.push_str("    }\n");
        js.push_str("});\n");

        js
    }
}

/// Test result dashboard generator
pub struct TestDashboardGenerator {
    output_dir: PathBuf,
}

impl TestDashboardGenerator {
    /// Create a new dashboard generator
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }

    /// Generate complete test dashboard
    pub fn generate_dashboard(&self, metrics: &[TestMetrics]) -> Result<(), Box<dyn std::error::Error>> {
        // Create output directory if it doesn't exist
        std::fs::create_dir_all(&self.output_dir)?;

        // Generate main report
        let report_path = self.output_dir.join("test_report.html");
        let generator = HtmlReportGenerator::new();
        generator.generate_report(metrics, &report_path)?;

        // Generate JSON data for programmatic access
        let json_path = self.output_dir.join("test_data.json");
        let json_data = serde_json::to_string_pretty(metrics)?;
        std::fs::write(json_path, json_data)?;

        // Generate summary CSV
        let csv_path = self.output_dir.join("test_summary.csv");
        self.generate_csv_summary(metrics, &csv_path)?;

        Ok(())
    }

    /// Generate CSV summary of test results
    fn generate_csv_summary(&self, metrics: &[TestMetrics], path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let mut csv = String::new();
        csv.push_str("test_name,success,execution_time_ms,memory_usage_mb,cpu_usage_percent,error_message\n");

        for metric in metrics {
            csv.push_str(&format!(
                "{},{},{:.3},{},{},{}\n",
                metric.test_name,
                metric.success,
                metric.execution_time.as_millis(),
                metric.memory_usage_bytes.map(|b| b as f64 / 1_048_576.0).unwrap_or(0.0),
                metric.cpu_usage_percent.unwrap_or(0.0),
                metric.error_message.as_deref().unwrap_or("")
            ));
        }

        std::fs::write(path, csv)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug_tools::TestMetrics;
    use std::time::Duration;

    #[test]
    fn test_html_report_generation() {
        let metrics = vec![
            TestMetrics {
                test_name: "test_example".to_string(),
                execution_time: Duration::from_millis(100),
                memory_usage_bytes: Some(1024 * 1024),
                cpu_usage_percent: Some(50.0),
                io_operations: None,
                assertions_count: 5,
                success: true,
                error_message: None,
                timestamp: chrono::Utc::now(),
            }
        ];

        let generator = HtmlReportGenerator::new();
        let temp_dir = std::env::temp_dir();
        let output_path = temp_dir.join("test_report.html");

        let result = generator.generate_report(&metrics, &output_path);
        assert!(result.is_ok());

        // Verify file was created
        assert!(output_path.exists());
    }

    #[test]
    fn test_dashboard_generation() {
        let metrics = vec![
            TestMetrics {
                test_name: "test_example".to_string(),
                execution_time: Duration::from_millis(100),
                memory_usage_bytes: Some(1024 * 1024),
                cpu_usage_percent: Some(50.0),
                io_operations: None,
                assertions_count: 5,
                success: true,
                error_message: None,
                timestamp: chrono::Utc::now(),
            }
        ];

        let temp_dir = std::env::temp_dir().join("test_dashboard");
        let generator = TestDashboardGenerator::new(temp_dir.clone());

        let result = generator.generate_dashboard(&metrics);
        assert!(result.is_ok());

        // Verify files were created
        assert!(temp_dir.join("test_report.html").exists());
        assert!(temp_dir.join("test_data.json").exists());
        assert!(temp_dir.join("test_summary.csv").exists());
    }
}
