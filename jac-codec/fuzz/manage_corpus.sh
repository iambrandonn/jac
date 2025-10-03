#!/bin/bash
# Corpus management script for JAC fuzzing

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Default values
CORPUS_DIR="corpus"
TARGET=""
ACTION=""

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

print_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

# Function to show usage
show_usage() {
    echo "Usage: $0 [OPTIONS] ACTION TARGET"
    echo ""
    echo "ACTIONS:"
    echo "  minimize    Minimize corpus by removing redundant entries"
    echo "  expand      Expand corpus with new test cases"
    echo "  stats       Show corpus statistics"
    echo "  clean       Clean corpus directory"
    echo "  generate    Generate new seed corpora"
    echo ""
    echo "TARGET: fuzz target (decode_block, varint, projection, compression, bitpack, all)"
    echo ""
    echo "OPTIONS:"
    echo "  -c, --corpus DIR    Corpus directory (default: corpus)"
    echo "  -h, --help          Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 minimize decode_block"
    echo "  $0 stats all"
    echo "  $0 generate varint"
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -c|--corpus)
            CORPUS_DIR="$2"
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
            if [[ -z "$ACTION" ]]; then
                ACTION="$1"
            elif [[ -z "$TARGET" ]]; then
                TARGET="$1"
            else
                print_error "Too many arguments"
                show_usage
                exit 1
            fi
            shift
            ;;
    esac
done

# Check if action and target are specified
if [[ -z "$ACTION" ]] || [[ -z "$TARGET" ]]; then
    print_error "Action and target must be specified"
    show_usage
    exit 1
fi

# Function to get corpus statistics
get_corpus_stats() {
    local target_name="$1"
    local corpus_path="$CORPUS_DIR/fuzz_$target_name"

    if [[ ! -d "$corpus_path" ]]; then
        print_warning "Corpus directory does not exist: $corpus_path"
        return 1
    fi

    local file_count=$(find "$corpus_path" -type f | wc -l)
    local total_size=$(du -sh "$corpus_path" 2>/dev/null | cut -f1)

    print_info "Corpus statistics for $target_name:"
    echo "  Files: $file_count"
    echo "  Size: $total_size"
    echo "  Path: $corpus_path"
}

# Function to minimize corpus
minimize_corpus() {
    local target_name="$1"
    local corpus_path="$CORPUS_DIR/fuzz_$target_name"

    if [[ ! -d "$corpus_path" ]]; then
        print_warning "Corpus directory does not exist: $corpus_path"
        return 1
    fi

    print_status "Minimizing corpus for $target_name..."

    # Create backup
    local backup_path="${corpus_path}_backup_$(date +%Y%m%d_%H%M%S)"
    cp -r "$corpus_path" "$backup_path"
    print_info "Backup created: $backup_path"

    # Simple minimization: remove duplicate files
    local original_count=$(find "$corpus_path" -type f | wc -l)

    # Remove files with identical content
    fdupes -r -d -N "$corpus_path" > /dev/null 2>&1 || true

    # Remove empty files
    find "$corpus_path" -type f -empty -delete

    local final_count=$(find "$corpus_path" -type f | wc -l)
    local removed=$((original_count - final_count))

    print_status "Removed $removed duplicate/empty files"
    print_info "Original: $original_count files, Final: $final_count files"
}

# Function to expand corpus
expand_corpus() {
    local target_name="$1"
    local corpus_path="$CORPUS_DIR/fuzz_$target_name"

    print_status "Expanding corpus for $target_name..."

    # Create corpus directory if it doesn't exist
    mkdir -p "$corpus_path"

    case "$target_name" in
        decode_block)
            # Add more JAC files with different characteristics
            print_info "Generating additional JAC files for decode_block..."

            # Create files with different compression levels
            for level in 1 3 6 9 15 19; do
                echo '[{"ts": 1234567890, "level": "info", "user": "test", "message": "compression level test"}]' | \
                    ./target/release/jac pack --zstd-level $level --output "$corpus_path/compression_level_$level.jac" /dev/stdin 2>/dev/null || true
            done

            # Create files with different block sizes
            for records in 1 10 100 1000; do
                python3 -c "
import json
data = [{'ts': 1234567890 + i, 'level': 'info', 'user': f'user{i}', 'message': f'message {i}'} for i in range($records)]
print(json.dumps(data))
" | ./target/release/jac pack --block-records $records --output "$corpus_path/block_size_$records.jac" /dev/stdin 2>/dev/null || true
            done
            ;;
        varint)
            # Add more varint test cases
            print_info "Generating additional varint test cases..."

            # Generate various varint patterns
            for i in {0..255}; do
                printf "\\x%02x" $i > "$corpus_path/byte_$i.bin"
            done

            # Generate multi-byte varints
            for i in {128..255}; do
                printf "\\x%02x\\x00" $i > "$corpus_path/multibyte_$i.bin"
            done
            ;;
        *)
            print_warning "No specific expansion rules for $target_name"
            ;;
    esac

    print_status "Corpus expansion completed for $target_name"
}

# Function to generate new seed corpora
generate_corpus() {
    local target_name="$1"
    local corpus_path="$CORPUS_DIR/fuzz_$target_name"

    print_status "Generating new seed corpus for $target_name..."

    # Create corpus directory
    mkdir -p "$corpus_path"

    case "$target_name" in
        decode_block)
            # Generate various JAC files
            print_info "Generating JAC files..."

            # Empty file
            echo '[]' | ./target/release/jac pack --output "$corpus_path/empty.jac" /dev/stdin 2>/dev/null || true

            # Single record
            echo '[{"ts": 0, "level": "info", "user": "test", "message": "single record"}]' | \
                ./target/release/jac pack --output "$corpus_path/single.jac" /dev/stdin 2>/dev/null || true

            # Multiple records with different types
            cat > /tmp/mixed_types.json << 'EOF'
[
  {"ts": 1234567890, "level": "info", "user": "alice", "message": "test", "count": 42, "active": true},
  {"ts": 1234567891, "level": "debug", "user": "bob", "message": "debug", "count": 0, "active": false},
  {"ts": 1234567892, "level": "error", "user": "carol", "message": "error", "count": -1, "active": null},
  {"ts": 1234567893, "level": "warn", "user": "dave", "message": "warn", "count": 999999, "active": true}
]
EOF
            ./target/release/jac pack --output "$corpus_path/mixed_types.jac" /tmp/mixed_types.json 2>/dev/null || true
            rm -f /tmp/mixed_types.json
            ;;
        varint)
            # Generate varint test cases
            print_info "Generating varint test cases..."

            # Basic values
            printf '\x00' > "$corpus_path/zero.bin"
            printf '\x01' > "$corpus_path/one.bin"
            printf '\x7F' > "$corpus_path/max_single.bin"
            printf '\x80\x01' > "$corpus_path/min_double.bin"
            printf '\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF\xFF\x01' > "$corpus_path/max_uleb128.bin"

            # Edge cases
            printf '\x80' > "$corpus_path/incomplete.bin"
            printf '\xFF' > "$corpus_path/incomplete_ff.bin"
            ;;
        *)
            print_warning "No generation rules for $target_name"
            ;;
    esac

    print_status "Seed corpus generation completed for $target_name"
}

# Function to clean corpus
clean_corpus() {
    local target_name="$1"
    local corpus_path="$CORPUS_DIR/fuzz_$target_name"

    if [[ "$target_name" == "all" ]]; then
        print_status "Cleaning all corpus directories..."
        rm -rf "$CORPUS_DIR"
        mkdir -p "$CORPUS_DIR"
    else
        print_status "Cleaning corpus for $target_name..."
        rm -rf "$corpus_path"
        mkdir -p "$corpus_path"
    fi

    print_status "Corpus cleaning completed"
}

# Main execution
case "$ACTION" in
    minimize)
        if [[ "$TARGET" == "all" ]]; then
            for target in decode_block varint projection compression bitpack; do
                minimize_corpus "$target"
            done
        else
            minimize_corpus "$TARGET"
        fi
        ;;
    expand)
        if [[ "$TARGET" == "all" ]]; then
            for target in decode_block varint projection compression bitpack; do
                expand_corpus "$target"
            done
        else
            expand_corpus "$TARGET"
        fi
        ;;
    stats)
        if [[ "$TARGET" == "all" ]]; then
            for target in decode_block varint projection compression bitpack; do
                get_corpus_stats "$target"
                echo ""
            done
        else
            get_corpus_stats "$TARGET"
        fi
        ;;
    clean)
        clean_corpus "$TARGET"
        ;;
    generate)
        if [[ "$TARGET" == "all" ]]; then
            for target in decode_block varint projection compression bitpack; do
                generate_corpus "$target"
            done
        else
            generate_corpus "$TARGET"
        fi
        ;;
    *)
        print_error "Unknown action: $ACTION"
        show_usage
        exit 1
        ;;
esac

print_status "Corpus management completed!"
