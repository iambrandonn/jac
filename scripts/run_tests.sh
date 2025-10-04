#!/bin/bash
# Test runner script for JAC that respects test categorization

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
RUN_UNIT=true
RUN_INTEGRATION=true
RUN_SLOW=false
RUN_STRESS=false
RUN_PERFORMANCE=false
RUN_HARDWARE=false
RUN_IGNORED=false
VERBOSE=false
HELP=false

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --unit)
            RUN_UNIT=true
            shift
            ;;
        --integration)
            RUN_INTEGRATION=true
            shift
            ;;
        --slow)
            RUN_SLOW=true
            shift
            ;;
        --stress)
            RUN_STRESS=true
            shift
            ;;
        --performance)
            RUN_PERFORMANCE=true
            shift
            ;;
        --hardware)
            RUN_HARDWARE=true
            shift
            ;;
        --ignored)
            RUN_IGNORED=true
            shift
            ;;
        --all)
            RUN_UNIT=true
            RUN_INTEGRATION=true
            RUN_SLOW=true
            RUN_STRESS=true
            RUN_PERFORMANCE=true
            RUN_HARDWARE=true
            RUN_IGNORED=true
            shift
            ;;
        --ci)
            RUN_UNIT=true
            RUN_INTEGRATION=true
            RUN_SLOW=false
            RUN_STRESS=false
            RUN_PERFORMANCE=false
            RUN_HARDWARE=false
            RUN_IGNORED=false
            shift
            ;;
        --nightly)
            RUN_UNIT=true
            RUN_INTEGRATION=true
            RUN_SLOW=true
            RUN_STRESS=true
            RUN_PERFORMANCE=true
            RUN_HARDWARE=true
            RUN_IGNORED=false
            shift
            ;;
        -v|--verbose)
            VERBOSE=true
            shift
            ;;
        -h|--help)
            HELP=true
            shift
            ;;
        *)
            echo "Unknown option $1"
            exit 1
            ;;
    esac
done

if [ "$HELP" = true ]; then
    echo "JAC Test Runner"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --unit          Run unit tests (default: true)"
    echo "  --integration   Run integration tests (default: true)"
    echo "  --slow          Run slow tests (default: false)"
    echo "  --stress        Run stress tests (default: false)"
    echo "  --performance   Run performance tests (default: false)"
    echo "  --hardware      Run hardware-specific tests (default: false)"
    echo "  --ignored       Run ignored tests (default: false)"
    echo "  --all           Run all tests"
    echo "  --ci            Run CI-appropriate tests (unit + integration)"
    echo "  --nightly       Run nightly tests (all except ignored)"
    echo "  -v, --verbose   Verbose output"
    echo "  -h, --help      Show this help"
    echo ""
    echo "Environment variables:"
    echo "  CI=true         Automatically use --ci mode"
    echo "  NIGHTLY=true    Automatically use --nightly mode"
    echo "  STRESS_TESTS=1  Automatically include --stress"
    echo "  PERFORMANCE_TESTS=1  Automatically include --performance"
    exit 0
fi

# Check environment variables
if [ "$CI" = "true" ]; then
    RUN_UNIT=true
    RUN_INTEGRATION=true
    RUN_SLOW=false
    RUN_STRESS=false
    RUN_PERFORMANCE=false
    RUN_HARDWARE=false
    RUN_IGNORED=false
fi

if [ "$NIGHTLY" = "true" ]; then
    RUN_UNIT=true
    RUN_INTEGRATION=true
    RUN_SLOW=true
    RUN_STRESS=true
    RUN_PERFORMANCE=true
    RUN_HARDWARE=true
    RUN_IGNORED=false
fi

if [ "$STRESS_TESTS" = "1" ]; then
    RUN_STRESS=true
fi

if [ "$PERFORMANCE_TESTS" = "1" ]; then
    RUN_PERFORMANCE=true
fi

# Function to run tests with filtering
run_tests() {
    local crate=$1
    local test_filter=$2
    local description=$3

    if [ "$VERBOSE" = true ]; then
        echo -e "${BLUE}Running $description tests for $crate...${NC}"
    fi

    if cargo test -p "$crate" $test_filter; then
        echo -e "${GREEN}✓ $description tests for $crate passed${NC}"
        return 0
    else
        echo -e "${RED}✗ $description tests for $crate failed${NC}"
        return 1
    fi
}

# Function to run ignored tests
run_ignored_tests() {
    local crate=$1
    local description=$2

    if [ "$VERBOSE" = true ]; then
        echo -e "${BLUE}Running ignored $description tests for $crate...${NC}"
    fi

    if cargo test -p "$crate" -- --ignored; then
        echo -e "${GREEN}✓ Ignored $description tests for $crate passed${NC}"
        return 0
    else
        echo -e "${RED}✗ Ignored $description tests for $crate failed${NC}"
        return 1
    fi
}

# Track overall success
OVERALL_SUCCESS=true

echo -e "${BLUE}JAC Test Runner${NC}"
echo "=================="
echo "Unit tests: $RUN_UNIT"
echo "Integration tests: $RUN_INTEGRATION"
echo "Slow tests: $RUN_SLOW"
echo "Stress tests: $RUN_STRESS"
echo "Performance tests: $RUN_PERFORMANCE"
echo "Hardware tests: $RUN_HARDWARE"
echo "Ignored tests: $RUN_IGNORED"
echo ""

# Run unit tests
if [ "$RUN_UNIT" = true ]; then
    run_tests "jac-format" "" "unit" || OVERALL_SUCCESS=false
    run_tests "jac-codec" "" "unit" || OVERALL_SUCCESS=false
    run_tests "jac-io" "" "unit" || OVERALL_SUCCESS=false
    run_tests "jac-cli" "" "unit" || OVERALL_SUCCESS=false
fi

# Run integration tests
if [ "$RUN_INTEGRATION" = true ]; then
    run_tests "jac-io" "--test integration_tests" "integration" || OVERALL_SUCCESS=false
    run_tests "jac-io" "--test concurrency_stress" "integration" || OVERALL_SUCCESS=false
    run_tests "jac-codec" "--test conformance" "integration" || OVERALL_SUCCESS=false
    run_tests "jac-codec" "--test cross_platform_compatibility" "integration" || OVERALL_SUCCESS=false
fi

# Run slow tests
if [ "$RUN_SLOW" = true ]; then
    run_ignored_tests "jac-io" "slow" || OVERALL_SUCCESS=false
fi

# Run stress tests
if [ "$RUN_STRESS" = true ]; then
    run_ignored_tests "jac-io" "stress" || OVERALL_SUCCESS=false
fi

# Run performance tests
if [ "$RUN_PERFORMANCE" = true ]; then
    run_ignored_tests "jac-codec" "performance" || OVERALL_SUCCESS=false
fi

# Run hardware tests
if [ "$RUN_HARDWARE" = true ]; then
    run_ignored_tests "jac-format" "hardware" || OVERALL_SUCCESS=false
    run_ignored_tests "jac-codec" "hardware" || OVERALL_SUCCESS=false
    run_ignored_tests "jac-io" "hardware" || OVERALL_SUCCESS=false
fi

# Run ignored tests
if [ "$RUN_IGNORED" = true ]; then
    run_ignored_tests "jac-format" "ignored" || OVERALL_SUCCESS=false
    run_ignored_tests "jac-codec" "ignored" || OVERALL_SUCCESS=false
    run_ignored_tests "jac-io" "ignored" || OVERALL_SUCCESS=false
    run_ignored_tests "jac-cli" "ignored" || OVERALL_SUCCESS=false
fi

# Summary
echo ""
if [ "$OVERALL_SUCCESS" = true ]; then
    echo -e "${GREEN}✓ All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}✗ Some tests failed!${NC}"
    exit 1
fi
