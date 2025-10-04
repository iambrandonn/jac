#!/bin/bash

# JAC Test Data Management Script
# This script manages test data generation, validation, and distribution

set -euo pipefail

# Configuration
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TESTDATA_DIR="$PROJECT_ROOT/testdata"
GENERATOR_SCRIPT="$SCRIPT_DIR/generate_test_data.py"
METADATA_DIR="$TESTDATA_DIR/metadata"
VERSION_FILE="$TESTDATA_DIR/VERSION"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
DEFAULT_SEED=42
DEFAULT_VERSION="1.0.0"
DEFAULT_FORMAT="ndjson"

# Function to print usage
usage() {
    echo "Usage: $0 [COMMAND] [OPTIONS]"
    echo ""
    echo "Commands:"
    echo "  generate    Generate test data"
    echo "  validate    Validate test data"
    echo "  list        List available test data"
    echo "  clean       Clean test data"
    echo "  compress    Compress test data"
    echo "  decompress  Decompress test data"
    echo "  metadata    Show metadata for test data"
    echo "  hash        Calculate hashes for test data"
    echo "  sync        Sync test data to remote storage"
    echo "  help        Show this help message"
    echo ""
    echo "Options:"
    echo "  --category CATEGORY    Test data category (unit|integration|performance|stress|conformance)"
    echo "  --size SIZE           Data size (small|medium|large|xlarge)"
    echo "  --count COUNT         Number of records to generate"
    echo "  --format FORMAT       Output format (json|ndjson|jsonl)"
    echo "  --seed SEED           Random seed for generation"
    echo "  --version VERSION     Data version"
    echo "  --compress FORMAT     Compression format (gzip|bzip2|lzma)"
    echo "  --output-dir DIR      Output directory"
    echo "  --verbose             Verbose output"
    echo ""
    echo "Examples:"
    echo "  $0 generate --category integration --size medium --count 1000"
    echo "  $0 validate --category integration --size medium"
    echo "  $0 list"
    echo "  $0 clean"
}

# Function to log messages
log() {
    local level="$1"
    shift
    local message="$*"
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')

    case "$level" in
        "INFO")
            echo -e "${BLUE}[INFO]${NC} $message"
            ;;
        "WARN")
            echo -e "${YELLOW}[WARN]${NC} $message"
            ;;
        "ERROR")
            echo -e "${RED}[ERROR]${NC} $message"
            ;;
        "SUCCESS")
            echo -e "${GREEN}[SUCCESS]${NC} $message"
            ;;
        *)
            echo -e "${BLUE}[INFO]${NC} $message"
            ;;
    esac
}

# Function to check dependencies
check_dependencies() {
    log "INFO" "Checking dependencies..."

    # Check Python
    if ! command -v python3 &> /dev/null; then
        log "ERROR" "Python 3 is required but not installed"
        exit 1
    fi

    # Check required Python packages
    if ! python3 -c "import json, random, string, time, argparse, os, sys, pathlib, hashlib, gzip, bz2, lzma" 2>/dev/null; then
        log "ERROR" "Required Python packages are missing"
        exit 1
    fi

    # Check Git
    if ! command -v git &> /dev/null; then
        log "WARN" "Git is not installed, version control features will be limited"
    fi

    log "SUCCESS" "All dependencies are available"
}

# Function to initialize test data directory
init_testdata_dir() {
    log "INFO" "Initializing test data directory..."

    # Create directory structure
    mkdir -p "$TESTDATA_DIR"/{unit,integration,performance,stress,conformance,generated,real,metadata}
    mkdir -p "$TESTDATA_DIR"/{unit,integration,performance,stress,conformance,generated,real}/{small,medium,large,xlarge}

    # Create version file
    echo "$DEFAULT_VERSION" > "$VERSION_FILE"

    # Create .gitignore
    cat > "$TESTDATA_DIR/.gitignore" << EOF
# Generated test data
generated/
real/

# Compressed files
*.gz
*.bz2
*.xz

# Temporary files
*.tmp
*.temp

# Large files (use Git LFS)
*.large
*.xlarge
EOF

    log "SUCCESS" "Test data directory initialized"
}

# Function to generate test data
generate_test_data() {
    local category="${1:-integration}"
    local size="${2:-medium}"
    local count="${3:-1000}"
    local format="${4:-$DEFAULT_FORMAT}"
    local seed="${5:-$DEFAULT_SEED}"
    local version="${6:-$DEFAULT_VERSION}"
    local compress="${7:-}"
    local output_dir="${8:-$TESTDATA_DIR}"

    log "INFO" "Generating test data: category=$category, size=$size, count=$count, format=$format"

    # Build command
    local cmd="python3 $GENERATOR_SCRIPT"
    cmd="$cmd --category $category"
    cmd="$cmd --size $size"
    cmd="$cmd --count $count"
    cmd="$cmd --format $format"
    cmd="$cmd --seed $seed"
    cmd="$cmd --version $version"
    cmd="$cmd --output-dir $output_dir"

    if [ -n "$compress" ]; then
        cmd="$cmd --compress $compress"
    fi

    # Execute command
    if eval "$cmd"; then
        log "SUCCESS" "Test data generated successfully"
    else
        log "ERROR" "Failed to generate test data"
        exit 1
    fi
}

# Function to validate test data
validate_test_data() {
    local category="${1:-integration}"
    local size="${2:-medium}"

    log "INFO" "Validating test data: category=$category, size=$size"

    local data_dir="$TESTDATA_DIR/$category/$size"

    if [ ! -d "$data_dir" ]; then
        log "ERROR" "Test data directory not found: $data_dir"
        exit 1
    fi

    local validation_errors=0

    # Validate JSON files
    for file in "$data_dir"/*.json; do
        if [ -f "$file" ]; then
            if python3 -m json.tool "$file" > /dev/null 2>&1; then
                log "INFO" "Valid JSON: $(basename "$file")"
            else
                log "ERROR" "Invalid JSON: $(basename "$file")"
                validation_errors=$((validation_errors + 1))
            fi
        fi
    done

    # Validate NDJSON files
    for file in "$data_dir"/*.ndjson; do
        if [ -f "$file" ]; then
            local line_count=0
            local error_count=0

            while IFS= read -r line; do
                line_count=$((line_count + 1))
                if ! echo "$line" | python3 -m json.tool > /dev/null 2>&1; then
                    error_count=$((error_count + 1))
                    log "ERROR" "Invalid JSON on line $line_count in $(basename "$file")"
                fi
            done < "$file"

            if [ $error_count -eq 0 ]; then
                log "INFO" "Valid NDJSON: $(basename "$file") ($line_count lines)"
            else
                log "ERROR" "Invalid NDJSON: $(basename "$file") ($error_count errors)"
                validation_errors=$((validation_errors + 1))
            fi
        fi
    done

    # Validate metadata files
    for file in "$data_dir"/*_metadata.json; do
        if [ -f "$file" ]; then
            if python3 -m json.tool "$file" > /dev/null 2>&1; then
                log "INFO" "Valid metadata: $(basename "$file")"
            else
                log "ERROR" "Invalid metadata: $(basename "$file")"
                validation_errors=$((validation_errors + 1))
            fi
        fi
    done

    if [ $validation_errors -eq 0 ]; then
        log "SUCCESS" "All test data validation passed"
    else
        log "ERROR" "Test data validation failed with $validation_errors errors"
        exit 1
    fi
}

# Function to list test data
list_test_data() {
    log "INFO" "Listing available test data..."

    if [ ! -d "$TESTDATA_DIR" ]; then
        log "ERROR" "Test data directory not found: $TESTDATA_DIR"
        exit 1
    fi

    echo "Test Data Directory: $TESTDATA_DIR"
    echo ""

    for category in unit integration performance stress conformance; do
        if [ -d "$TESTDATA_DIR/$category" ]; then
            echo "Category: $category"
            for size in small medium large xlarge; do
                if [ -d "$TESTDATA_DIR/$category/$size" ]; then
                    echo "  Size: $size"
                    local file_count=$(find "$TESTDATA_DIR/$category/$size" -type f | wc -l)
                    echo "    Files: $file_count"

                    # List files
                    for file in "$TESTDATA_DIR/$category/$size"/*; do
                        if [ -f "$file" ]; then
                            local file_size=$(du -h "$file" | cut -f1)
                            local file_name=$(basename "$file")
                            echo "      $file_name ($file_size)"
                        fi
                    done
                fi
            done
            echo ""
        fi
    done
}

# Function to clean test data
clean_test_data() {
    local category="${1:-}"
    local size="${2:-}"

    log "INFO" "Cleaning test data..."

    if [ -n "$category" ] && [ -n "$size" ]; then
        # Clean specific category and size
        local data_dir="$TESTDATA_DIR/$category/$size"
        if [ -d "$data_dir" ]; then
            rm -rf "$data_dir"/*
            log "SUCCESS" "Cleaned $category/$size test data"
        else
            log "WARN" "Test data directory not found: $data_dir"
        fi
    else
        # Clean all test data
        if [ -d "$TESTDATA_DIR" ]; then
            rm -rf "$TESTDATA_DIR"/*
            log "SUCCESS" "Cleaned all test data"
        else
            log "WARN" "Test data directory not found: $TESTDATA_DIR"
        fi
    fi
}

# Function to compress test data
compress_test_data() {
    local category="${1:-integration}"
    local size="${2:-medium}"
    local format="${3:-gzip}"

    log "INFO" "Compressing test data: category=$category, size=$size, format=$format"

    local data_dir="$TESTDATA_DIR/$category/$size"

    if [ ! -d "$data_dir" ]; then
        log "ERROR" "Test data directory not found: $data_dir"
        exit 1
    fi

    local compressed_count=0

    for file in "$data_dir"/*.json "$data_dir"/*.ndjson; do
        if [ -f "$file" ]; then
            local compressed_file="${file}.${format}"
            if [ "$format" = "gzip" ]; then
                gzip -c "$file" > "$compressed_file"
            elif [ "$format" = "bzip2" ]; then
                bzip2 -c "$file" > "$compressed_file"
            elif [ "$format" = "lzma" ]; then
                lzma -c "$file" > "$compressed_file"
            else
                log "ERROR" "Unsupported compression format: $format"
                exit 1
            fi
            compressed_count=$((compressed_count + 1))
            log "INFO" "Compressed: $(basename "$file") -> $(basename "$compressed_file")"
        fi
    done

    log "SUCCESS" "Compressed $compressed_count files"
}

# Function to decompress test data
decompress_test_data() {
    local category="${1:-integration}"
    local size="${2:-medium}"

    log "INFO" "Decompressing test data: category=$category, size=$size"

    local data_dir="$TESTDATA_DIR/$category/$size"

    if [ ! -d "$data_dir" ]; then
        log "ERROR" "Test data directory not found: $data_dir"
        exit 1
    fi

    local decompressed_count=0

    for file in "$data_dir"/*.gz "$data_dir"/*.bz2 "$data_dir"/*.xz; do
        if [ -f "$file" ]; then
            local decompressed_file="${file%.*}"
            if [[ "$file" == *.gz ]]; then
                gunzip -c "$file" > "$decompressed_file"
            elif [[ "$file" == *.bz2 ]]; then
                bunzip2 -c "$file" > "$decompressed_file"
            elif [[ "$file" == *.xz ]]; then
                unxz -c "$file" > "$decompressed_file"
            fi
            decompressed_count=$((decompressed_count + 1))
            log "INFO" "Decompressed: $(basename "$file") -> $(basename "$decompressed_file")"
        fi
    done

    log "SUCCESS" "Decompressed $decompressed_count files"
}

# Function to show metadata
show_metadata() {
    local category="${1:-integration}"
    local size="${2:-medium}"

    log "INFO" "Showing metadata: category=$category, size=$size"

    local data_dir="$TESTDATA_DIR/$category/$size"

    if [ ! -d "$data_dir" ]; then
        log "ERROR" "Test data directory not found: $data_dir"
        exit 1
    fi

    for file in "$data_dir"/*_metadata.json; do
        if [ -f "$file" ]; then
            echo "Metadata: $(basename "$file")"
            echo "----------------------------------------"
            python3 -m json.tool "$file"
            echo ""
        fi
    done
}

# Function to calculate hashes
calculate_hashes() {
    local category="${1:-integration}"
    local size="${2:-medium}"

    log "INFO" "Calculating hashes: category=$category, size=$size"

    local data_dir="$TESTDATA_DIR/$category/$size"

    if [ ! -d "$data_dir" ]; then
        log "ERROR" "Test data directory not found: $data_dir"
        exit 1
    fi

    local hash_file="$data_dir/hashes.txt"

    echo "# Test Data Hashes" > "$hash_file"
    echo "# Generated: $(date)" >> "$hash_file"
    echo "" >> "$hash_file"

    for file in "$data_dir"/*; do
        if [ -f "$file" ] && [[ "$file" != *.txt ]]; then
            local hash=$(sha256sum "$file" | cut -d' ' -f1)
            local file_name=$(basename "$file")
            echo "$hash  $file_name" >> "$hash_file"
            log "INFO" "Hash calculated: $file_name"
        fi
    done

    log "SUCCESS" "Hashes saved to: $hash_file"
}

# Function to sync test data
sync_test_data() {
    log "INFO" "Syncing test data to remote storage..."

    # This is a placeholder for actual sync implementation
    # In a real implementation, this would sync to cloud storage, Git LFS, etc.

    log "WARN" "Sync functionality not implemented yet"
    log "INFO" "Test data is available locally at: $TESTDATA_DIR"
}

# Main function
main() {
    local command="${1:-help}"
    shift || true

    # Parse command line arguments
    local category=""
    local size=""
    local count=""
    local format=""
    local seed=""
    local version=""
    local compress=""
    local output_dir=""
    local verbose=false

    while [[ $# -gt 0 ]]; do
        case $1 in
            --category)
                category="$2"
                shift 2
                ;;
            --size)
                size="$2"
                shift 2
                ;;
            --count)
                count="$2"
                shift 2
                ;;
            --format)
                format="$2"
                shift 2
                ;;
            --seed)
                seed="$2"
                shift 2
                ;;
            --version)
                version="$2"
                shift 2
                ;;
            --compress)
                compress="$2"
                shift 2
                ;;
            --output-dir)
                output_dir="$2"
                shift 2
                ;;
            --verbose)
                verbose=true
                shift
                ;;
            *)
                log "ERROR" "Unknown option: $1"
                usage
                exit 1
                ;;
        esac
    done

    # Set defaults
    category="${category:-integration}"
    size="${size:-medium}"
    count="${count:-1000}"
    format="${format:-$DEFAULT_FORMAT}"
    seed="${seed:-$DEFAULT_SEED}"
    version="${version:-$DEFAULT_VERSION}"
    output_dir="${output_dir:-$TESTDATA_DIR}"

    # Check dependencies
    check_dependencies

    # Initialize test data directory if it doesn't exist
    if [ ! -d "$TESTDATA_DIR" ]; then
        init_testdata_dir
    fi

    # Execute command
    case "$command" in
        "generate")
            generate_test_data "$category" "$size" "$count" "$format" "$seed" "$version" "$compress" "$output_dir"
            ;;
        "validate")
            validate_test_data "$category" "$size"
            ;;
        "list")
            list_test_data
            ;;
        "clean")
            clean_test_data "$category" "$size"
            ;;
        "compress")
            compress_test_data "$category" "$size" "$compress"
            ;;
        "decompress")
            decompress_test_data "$category" "$size"
            ;;
        "metadata")
            show_metadata "$category" "$size"
            ;;
        "hash")
            calculate_hashes "$category" "$size"
            ;;
        "sync")
            sync_test_data
            ;;
        "help"|*)
            usage
            ;;
    esac
}

# Run main function
main "$@"
