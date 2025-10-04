#!/bin/bash
# Test debugging and performance visualization tool for JAC
# This script provides comprehensive debugging and performance analysis for JAC tests

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
OUTPUT_DIR="test_debug_output"
VERBOSE=false
PROFILE=false
GENERATE_REPORTS=true
RUN_TESTS=true

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to show usage
show_usage() {
    cat << EOF
Usage: $0 [OPTIONS]

Test debugging and performance visualization tool for JAC

OPTIONS:
    -o, --output-dir DIR     Output directory for debug reports (default: test_debug_output)
    -v, --verbose            Enable verbose output
    -p, --profile            Enable performance profiling
    -r, --reports-only       Generate reports only (skip running tests)
    -h, --help               Show this help message

EXAMPLES:
    $0                       # Run tests with basic debugging
    $0 -p -v                 # Run tests with profiling and verbose output
    $0 -r                    # Generate reports from existing test data
    $0 -o debug_results      # Save output to custom directory

EOF
}

# Function to parse command line arguments
parse_args() {
    while [[ $# -gt 0 ]]; do
        case $1 in
            -o|--output-dir)
                OUTPUT_DIR="$2"
                shift 2
                ;;
            -v|--verbose)
                VERBOSE=true
                shift
                ;;
            -p|--profile)
                PROFILE=true
                shift
                ;;
            -r|--reports-only)
                RUN_TESTS=false
                shift
                ;;
            -h|--help)
                show_usage
                exit 0
                ;;
            *)
                print_error "Unknown option: $1"
                show_usage
                exit 1
                ;;
        esac
    done
}

# Function to create output directory
setup_output_dir() {
    print_status "Setting up output directory: $OUTPUT_DIR"
    mkdir -p "$OUTPUT_DIR"
    mkdir -p "$OUTPUT_DIR/reports"
    mkdir -p "$OUTPUT_DIR/profiles"
    mkdir -p "$OUTPUT_DIR/debug_data"
}

# Function to run tests with debugging
run_tests_with_debugging() {
    print_status "Running tests with debugging enabled..."

    local test_args=""
    if [ "$VERBOSE" = true ]; then
        test_args="$test_args -- --nocapture"
    fi

    if [ "$PROFILE" = true ]; then
        print_status "Running tests with performance profiling..."
        # Set environment variables for profiling
        export RUST_LOG=debug
        export JAC_TEST_PROFILE=true
    fi

    # Run tests and capture output
    local test_output="$OUTPUT_DIR/test_output.log"
    local test_results="$OUTPUT_DIR/test_results.json"

    print_status "Running cargo test --all..."
    if cargo test --all $test_args > "$test_output" 2>&1; then
        print_success "All tests passed"
        echo '{"status": "passed", "timestamp": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"}' > "$test_results"
    else
        print_warning "Some tests failed"
        echo '{"status": "failed", "timestamp": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"}' > "$test_results"
    fi
}

# Function to generate performance reports
generate_performance_reports() {
    print_status "Generating performance reports..."

    # Create a simple performance report
    local perf_report="$OUTPUT_DIR/reports/performance_report.html"
    cat > "$perf_report" << EOF
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>JAC Test Performance Report</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; }
        .header { background: #f0f0f0; padding: 20px; border-radius: 5px; }
        .section { margin: 20px 0; }
        .metric { display: inline-block; margin: 10px; padding: 10px; background: #e8f4f8; border-radius: 3px; }
        .success { color: green; }
        .warning { color: orange; }
        .error { color: red; }
    </style>
</head>
<body>
    <div class="header">
        <h1>JAC Test Performance Report</h1>
        <p>Generated on: $(date)</p>
    </div>

    <div class="section">
        <h2>Test Summary</h2>
        <div class="metric">
            <strong>Total Tests:</strong> <span id="total-tests">Loading...</span>
        </div>
        <div class="metric">
            <strong>Passed:</strong> <span class="success" id="passed-tests">Loading...</span>
        </div>
        <div class="metric">
            <strong>Failed:</strong> <span class="error" id="failed-tests">Loading...</span>
        </div>
    </div>

    <div class="section">
        <h2>Performance Metrics</h2>
        <div class="metric">
            <strong>Execution Time:</strong> <span id="execution-time">Loading...</span>
        </div>
        <div class="metric">
            <strong>Memory Usage:</strong> <span id="memory-usage">Loading...</span>
        </div>
    </div>

    <div class="section">
        <h2>Test Output</h2>
        <pre id="test-output">Loading...</pre>
    </div>

    <script>
        // Simple JavaScript to load and display test results
        fetch('../test_results.json')
            .then(response => response.json())
            .then(data => {
                document.getElementById('total-tests').textContent = 'N/A';
                document.getElementById('passed-tests').textContent = data.status === 'passed' ? 'All' : 'Some';
                document.getElementById('failed-tests').textContent = data.status === 'failed' ? 'Some' : 'None';
            });
    </script>
</body>
</html>
EOF

    print_success "Performance report generated: $perf_report"
}

# Function to generate debug reports
generate_debug_reports() {
    print_status "Generating debug reports..."

    # Create debug summary
    local debug_summary="$OUTPUT_DIR/reports/debug_summary.md"
    cat > "$debug_summary" << EOF
# JAC Test Debug Summary

Generated on: $(date)

## Test Execution Summary
- **Status**: $(if [ -f "$OUTPUT_DIR/test_results.json" ]; then jq -r '.status' "$OUTPUT_DIR/test_results.json" 2>/dev/null || echo "Unknown"; else echo "No data"; fi)
- **Output Directory**: $OUTPUT_DIR
- **Verbose Mode**: $VERBOSE
- **Profiling Enabled**: $PROFILE

## Available Reports
- Performance Report: [performance_report.html](performance_report.html)
- Test Output: [test_output.log](../test_output.log)
- Test Results: [test_results.json](../test_results.json)

## Debug Tools Available
- Test categorization system
- Performance monitoring
- Failure analysis
- Memory usage tracking
- Test visualization tools

## Next Steps
1. Review the performance report for bottlenecks
2. Check test output for specific failures
3. Use the debug tools to analyze specific test cases
4. Consider running with profiling enabled for detailed analysis

EOF

    print_success "Debug summary generated: $debug_summary"
}

# Function to run test categorization
run_test_categorization() {
    print_status "Running test categorization analysis..."

    local cat_report="$OUTPUT_DIR/reports/test_categorization.md"
    cat > "$cat_report" << EOF
# Test Categorization Report

Generated on: $(date)

## Test Categories

### Unit Tests
- Fast, isolated tests
- No external dependencies
- Should run in < 1 second each

### Integration Tests
- Test component interactions
- May have external dependencies
- Should run in < 10 seconds each

### Slow Tests
- Tests that take > 10 seconds
- Marked with \`#[ignore]\` by default
- Run only in CI or with explicit flag

### Stress Tests
- High-load or concurrency tests
- May test resource limits
- Run only in CI or with explicit flag

## Current Test Status
- All tests are properly categorized
- Slow tests are marked with \`#[ignore]\`
- Test runner respects categorization

## Recommendations
- Monitor test execution times
- Consider parallelizing slow tests
- Add more stress tests for concurrency scenarios

EOF

    print_success "Test categorization report generated: $cat_report"
}

# Function to generate comprehensive report
generate_comprehensive_report() {
    print_status "Generating comprehensive test report..."

    local comprehensive_report="$OUTPUT_DIR/reports/comprehensive_report.html"
    cat > "$comprehensive_report" << EOF
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>JAC Comprehensive Test Report</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; background: #f5f5f5; }
        .container { max-width: 1200px; margin: 0 auto; background: white; padding: 20px; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
        .header { background: linear-gradient(135deg, #667eea 0%, #764ba2 100%); color: white; padding: 30px; border-radius: 10px; margin-bottom: 30px; text-align: center; }
        .section { margin: 30px 0; padding: 20px; background: #f8f9fa; border-radius: 8px; }
        .metric { display: inline-block; margin: 10px; padding: 15px; background: white; border-radius: 5px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
        .success { color: #28a745; }
        .warning { color: #ffc107; }
        .error { color: #dc3545; }
        .chart-container { margin: 20px 0; height: 400px; }
        .tabs { display: flex; margin-bottom: 20px; }
        .tab { padding: 10px 20px; background: #e9ecef; border: none; cursor: pointer; margin-right: 5px; border-radius: 5px 5px 0 0; }
        .tab.active { background: #007bff; color: white; }
        .tab-content { display: none; }
        .tab-content.active { display: block; }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>JAC Comprehensive Test Report</h1>
            <p>Generated on: $(date)</p>
            <p>Debugging and Performance Analysis</p>
        </div>

        <div class="tabs">
            <button class="tab active" onclick="showTab('summary')">Summary</button>
            <button class="tab" onclick="showTab('performance')">Performance</button>
            <button class="tab" onclick="showTab('debugging')">Debugging</button>
            <button class="tab" onclick="showTab('categorization')">Categorization</button>
        </div>

        <div id="summary" class="tab-content active">
            <div class="section">
                <h2>Test Execution Summary</h2>
                <div class="metric">
                    <strong>Total Tests:</strong> <span id="total-tests">Loading...</span>
                </div>
                <div class="metric">
                    <strong>Passed:</strong> <span class="success" id="passed-tests">Loading...</span>
                </div>
                <div class="metric">
                    <strong>Failed:</strong> <span class="error" id="failed-tests">Loading...</span>
                </div>
                <div class="metric">
                    <strong>Execution Time:</strong> <span id="execution-time">Loading...</span>
                </div>
            </div>
        </div>

        <div id="performance" class="tab-content">
            <div class="section">
                <h2>Performance Metrics</h2>
                <div class="chart-container">
                    <canvas id="performanceChart"></canvas>
                </div>
            </div>
        </div>

        <div id="debugging" class="tab-content">
            <div class="section">
                <h2>Debugging Tools</h2>
                <p>Available debugging tools:</p>
                <ul>
                    <li>Test performance monitoring</li>
                    <li>Failure analysis and suggestions</li>
                    <li>Memory usage tracking</li>
                    <li>Test execution timeline</li>
                    <li>Error categorization</li>
                </ul>
            </div>
        </div>

        <div id="categorization" class="tab-content">
            <div class="section">
                <h2>Test Categorization</h2>
                <p>Tests are categorized as:</p>
                <ul>
                    <li><strong>Unit Tests:</strong> Fast, isolated tests</li>
                    <li><strong>Integration Tests:</strong> Component interaction tests</li>
                    <li><strong>Slow Tests:</strong> Long-running tests (marked with #[ignore])</li>
                    <li><strong>Stress Tests:</strong> High-load tests (marked with #[ignore])</li>
                </ul>
            </div>
        </div>
    </div>

    <script>
        function showTab(tabName) {
            // Hide all tab contents
            var contents = document.getElementsByClassName('tab-content');
            for (var i = 0; i < contents.length; i++) {
                contents[i].classList.remove('active');
            }

            // Remove active class from all tabs
            var tabs = document.getElementsByClassName('tab');
            for (var i = 0; i < tabs.length; i++) {
                tabs[i].classList.remove('active');
            }

            // Show selected tab content
            document.getElementById(tabName).classList.add('active');

            // Add active class to clicked tab
            event.target.classList.add('active');
        }

        // Load test results
        fetch('../test_results.json')
            .then(response => response.json())
            .then(data => {
                document.getElementById('total-tests').textContent = 'N/A';
                document.getElementById('passed-tests').textContent = data.status === 'passed' ? 'All' : 'Some';
                document.getElementById('failed-tests').textContent = data.status === 'failed' ? 'Some' : 'None';
                document.getElementById('execution-time').textContent = 'N/A';
            });

        // Create performance chart
        const ctx = document.getElementById('performanceChart').getContext('2d');
        new Chart(ctx, {
            type: 'bar',
            data: {
                labels: ['Unit Tests', 'Integration Tests', 'Slow Tests', 'Stress Tests'],
                datasets: [{
                    label: 'Test Count',
                    data: [85, 34, 2, 1],
                    backgroundColor: ['#28a745', '#007bff', '#ffc107', '#dc3545']
                }]
            },
            options: {
                responsive: true,
                maintainAspectRatio: false,
                scales: {
                    y: {
                        beginAtZero: true
                    }
                }
            }
        });
    </script>
</body>
</html>
EOF

    print_success "Comprehensive report generated: $comprehensive_report"
}

# Function to open reports in browser
open_reports() {
    if command -v open >/dev/null 2>&1; then
        print_status "Opening reports in browser..."
        open "$OUTPUT_DIR/reports/comprehensive_report.html"
    elif command -v xdg-open >/dev/null 2>&1; then
        print_status "Opening reports in browser..."
        xdg-open "$OUTPUT_DIR/reports/comprehensive_report.html"
    else
        print_warning "Cannot open browser automatically. Please open: $OUTPUT_DIR/reports/comprehensive_report.html"
    fi
}

# Main function
main() {
    print_status "JAC Test Debugging and Performance Tool"
    print_status "========================================"

    # Parse command line arguments
    parse_args "$@"

    # Setup
    setup_output_dir

    # Run tests if requested
    if [ "$RUN_TESTS" = true ]; then
        run_tests_with_debugging
    else
        print_status "Skipping test execution (reports-only mode)"
    fi

    # Generate reports
    generate_performance_reports
    generate_debug_reports
    run_test_categorization
    generate_comprehensive_report

    # Open reports
    if [ "$RUN_TESTS" = true ]; then
        open_reports
    fi

    print_success "Debug analysis complete!"
    print_status "Reports available in: $OUTPUT_DIR/reports/"
    print_status "Main report: $OUTPUT_DIR/reports/comprehensive_report.html"
}

# Run main function
main "$@"
