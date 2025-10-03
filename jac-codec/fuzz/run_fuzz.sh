#!/bin/bash
# Fuzz runner script for JAC codec fuzzing

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Default values
TARGET=""
TIME="60"
CORPUS_DIR="corpus"
CRASH_DIR="crashes"

# Function to print colored output
print_status() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to show usage
show_usage() {
    echo "Usage: $0 [OPTIONS] TARGET"
    echo ""
    echo "TARGET: fuzz target to run (decode_block, varint, projection, compression, bitpack, all)"
    echo ""
    echo "OPTIONS:"
    echo "  -t, --time SECONDS    Maximum time to run fuzzing (default: 60)"
    echo "  -c, --corpus DIR      Corpus directory (default: corpus)"
    echo "  -o, --output DIR      Output directory for crashes (default: crashes)"
    echo "  -h, --help           Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 decode_block -t 300"
    echo "  $0 all -t 600"
    echo "  $0 varint --corpus my_corpus"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -t|--time)
            TIME="$2"
            shift 2
            ;;
        -c|--corpus)
            CORPUS_DIR="$2"
            shift 2
            ;;
        -o|--output)
            CRASH_DIR="$2"
            shift 2
            ;;
        -h|--help)
            show_usage
            exit 0
            ;;
        -*)
            print_error "Unknown option $1"
            show_usage
            exit 1
            ;;
        *)
            if [[ -z "$TARGET" ]]; then
                TARGET="$1"
            else
                print_error "Multiple targets specified"
                show_usage
                exit 1
            fi
            shift
            ;;
    esac
done

# Check if target is specified
if [[ -z "$TARGET" ]]; then
    print_error "No target specified"
    show_usage
    exit 1
fi

# Check if cargo-fuzz is installed
if ! command -v cargo-fuzz &> /dev/null; then
    print_error "cargo-fuzz is not installed. Install it with:"
    echo "  cargo install cargo-fuzz"
    exit 1
fi

# Create output directory
mkdir -p "$CRASH_DIR"

# Function to run a specific fuzz target
run_fuzz_target() {
    local target_name="$1"
    local corpus_path="$CORPUS_DIR/fuzz_$target_name"
    local crash_path="$CRASH_DIR/fuzz_$target_name"

    print_status "Running fuzz target: $target_name"
    print_status "Corpus: $corpus_path"
    print_status "Crashes: $crash_path"
    print_status "Time limit: ${TIME}s"

    # Create corpus directory if it doesn't exist
    mkdir -p "$corpus_path"

    # Run the fuzzer
    cargo fuzz run "fuzz_$target_name" \
        --corpus "$corpus_path" \
        --artifacts "$crash_path" \
        -- -max_total_time="$TIME" \
        -print_final_stats=1 \
        -print_corpus_stats=1

    # Check for crashes
    local crash_count=$(find "$crash_path" -name "crash-*" 2>/dev/null | wc -l)
    if [[ $crash_count -gt 0 ]]; then
        print_warning "Found $crash_count crash(es) in $target_name"
        print_status "Crashes saved to: $crash_path"
    else
        print_status "No crashes found in $target_name"
    fi
}

# Function to run all fuzz targets
run_all_targets() {
    local targets=("decode_block" "varint" "projection" "compression" "bitpack")

    for target in "${targets[@]}"; do
        run_fuzz_target "$target"
        echo ""
    done
}

# Main execution
case "$TARGET" in
    all)
        run_all_targets
        ;;
    decode_block|varint|projection|compression|bitpack)
        run_fuzz_target "$TARGET"
        ;;
    *)
        print_error "Unknown target: $TARGET"
        print_status "Available targets: decode_block, varint, projection, compression, bitpack, all"
        exit 1
        ;;
esac

print_status "Fuzzing completed!"
