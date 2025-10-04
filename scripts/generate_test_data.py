#!/usr/bin/env python3
"""
JAC Test Data Generator

This script generates comprehensive test data for the JAC library,
including unit tests, integration tests, performance tests, and stress tests.
"""

import json
import random
import string
import time
import argparse
import os
import sys
from datetime import datetime, timedelta
from typing import Dict, List, Any, Optional
from pathlib import Path
import hashlib
import gzip
import bz2
import lzma

# Configuration
DEFAULT_OUTPUT_DIR = "testdata"
DEFAULT_SEED = 42
DEFAULT_VERSION = "1.0.0"

# Data generation parameters
FIELD_TYPES = ["string", "integer", "float", "boolean", "null", "object", "array"]
STRING_LENGTHS = [1, 10, 100, 1000, 10000, 100000]
ARRAY_SIZES = [1, 10, 100, 1000, 10000]
OBJECT_DEPTHS = [1, 2, 3, 4, 5]

class TestDataGenerator:
    """Main test data generator class."""

    def __init__(self, seed: int = DEFAULT_SEED, version: str = DEFAULT_VERSION):
        """Initialize the generator with a seed and version."""
        self.seed = seed
        self.version = version
        random.seed(seed)

    def generate_string(self, length: int) -> str:
        """Generate a random string of specified length."""
        if length == 0:
            return ""
        return ''.join(random.choices(string.ascii_letters + string.digits + string.punctuation, k=length))

    def generate_unicode_string(self, length: int) -> str:
        """Generate a random Unicode string of specified length."""
        if length == 0:
            return ""
        # Generate random Unicode characters in various ranges
        chars = []
        for _ in range(length):
            # Randomly choose from different Unicode ranges
            if random.random() < 0.3:
                # ASCII range
                chars.append(chr(random.randint(32, 126)))
            elif random.random() < 0.5:
                # Latin-1 range
                chars.append(chr(random.randint(128, 255)))
            elif random.random() < 0.7:
                # Basic Multilingual Plane
                chars.append(chr(random.randint(0x100, 0xFFFF)))
            else:
                # Supplementary Multilingual Plane
                chars.append(chr(random.randint(0x10000, 0x10FFFF)))
        return ''.join(chars)

    def generate_integer(self, min_val: int = -1000000, max_val: int = 1000000) -> int:
        """Generate a random integer in the specified range."""
        return random.randint(min_val, max_val)

    def generate_float(self, min_val: float = -1000000.0, max_val: float = 1000000.0) -> float:
        """Generate a random float in the specified range."""
        return random.uniform(min_val, max_val)

    def generate_boolean(self) -> bool:
        """Generate a random boolean value."""
        return random.choice([True, False])

    def generate_null(self) -> None:
        """Generate a null value."""
        return None

    def generate_array(self, size: int, field_type: str) -> List[Any]:
        """Generate a random array of specified size and field type."""
        if size == 0:
            return []

        array = []
        for _ in range(size):
            if field_type == "string":
                array.append(self.generate_string(random.choice(STRING_LENGTHS)))
            elif field_type == "integer":
                array.append(self.generate_integer())
            elif field_type == "float":
                array.append(self.generate_float())
            elif field_type == "boolean":
                array.append(self.generate_boolean())
            elif field_type == "null":
                array.append(self.generate_null())
            elif field_type == "object":
                array.append(self.generate_object(random.randint(1, 5)))
            elif field_type == "array":
                array.append(self.generate_array(random.randint(1, 10), "string"))

        return array

    def generate_object(self, depth: int) -> Dict[str, Any]:
        """Generate a random object with specified depth."""
        if depth <= 0:
            return {}

        obj = {}
        field_count = random.randint(1, 10)

        for i in range(field_count):
            field_name = f"field_{i}"
            field_type = random.choice(FIELD_TYPES)

            if field_type == "string":
                obj[field_name] = self.generate_string(random.choice(STRING_LENGTHS))
            elif field_type == "integer":
                obj[field_name] = self.generate_integer()
            elif field_type == "float":
                obj[field_name] = self.generate_float()
            elif field_type == "boolean":
                obj[field_name] = self.generate_boolean()
            elif field_type == "null":
                obj[field_name] = self.generate_null()
            elif field_type == "object" and depth > 1:
                obj[field_name] = self.generate_object(depth - 1)
            elif field_type == "array":
                obj[field_name] = self.generate_array(random.choice(ARRAY_SIZES), "string")

        return obj

    def generate_record(self, field_types: List[str]) -> Dict[str, Any]:
        """Generate a single record with specified field types."""
        record = {}

        for i, field_type in enumerate(field_types):
            field_name = f"field_{i}"

            if field_type == "string":
                record[field_name] = self.generate_string(random.choice(STRING_LENGTHS))
            elif field_type == "integer":
                record[field_name] = self.generate_integer()
            elif field_type == "float":
                record[field_name] = self.generate_float()
            elif field_type == "boolean":
                record[field_name] = self.generate_boolean()
            elif field_type == "null":
                record[field_name] = self.generate_null()
            elif field_type == "object":
                record[field_name] = self.generate_object(random.randint(1, 3))
            elif field_type == "array":
                record[field_name] = self.generate_array(random.choice(ARRAY_SIZES), "string")

        return record

    def generate_time_series_data(self, count: int, start_time: datetime) -> List[Dict[str, Any]]:
        """Generate time series data with timestamps."""
        records = []
        current_time = start_time

        for i in range(count):
            record = {
                "timestamp": current_time.isoformat(),
                "value": self.generate_float(),
                "category": random.choice(["A", "B", "C", "D"]),
                "level": random.choice(["DEBUG", "INFO", "WARN", "ERROR"]),
                "message": self.generate_string(random.randint(10, 100))
            }
            records.append(record)
            current_time += timedelta(seconds=random.randint(1, 3600))

        return records

    def generate_log_data(self, count: int) -> List[Dict[str, Any]]:
        """Generate realistic log data."""
        records = []

        for i in range(count):
            record = {
                "timestamp": datetime.now().isoformat(),
                "level": random.choice(["DEBUG", "INFO", "WARN", "ERROR", "FATAL"]),
                "logger": f"com.example.{random.choice(['service', 'controller', 'repository'])}",
                "message": self.generate_string(random.randint(20, 200)),
                "thread": f"thread-{random.randint(1, 10)}",
                "user_id": random.randint(1, 10000),
                "session_id": self.generate_string(32),
                "request_id": self.generate_string(16),
                "duration_ms": random.randint(1, 5000),
                "memory_mb": random.randint(100, 10000)
            }
            records.append(record)

        return records

    def generate_compression_test_data(self, count: int) -> List[Dict[str, Any]]:
        """Generate data optimized for compression testing."""
        records = []

        # Generate repetitive data for good compression
        base_string = self.generate_string(1000)
        base_object = self.generate_object(2)

        for i in range(count):
            record = {
                "id": i,
                "base_string": base_string,  # Same string for all records
                "base_object": base_object,  # Same object for all records
                "varying_field": self.generate_string(10),  # Small variation
                "timestamp": datetime.now().isoformat()
            }
            records.append(record)

        return records

    def generate_edge_case_data(self) -> List[Dict[str, Any]]:
        """Generate edge case data for testing."""
        records = []

        # Empty record
        records.append({})

        # Record with all null values
        records.append({
            "field1": None,
            "field2": None,
            "field3": None
        })

        # Record with empty strings
        records.append({
            "field1": "",
            "field2": "",
            "field3": ""
        })

        # Record with very long strings
        records.append({
            "field1": self.generate_string(100000),
            "field2": self.generate_string(100000),
            "field3": self.generate_string(100000)
        })

        # Record with very large numbers
        records.append({
            "field1": 2**63 - 1,
            "field2": -2**63,
            "field3": 1.7976931348623157e+308
        })

        # Record with Unicode strings
        records.append({
            "field1": self.generate_unicode_string(100),
            "field2": self.generate_unicode_string(100),
            "field3": self.generate_unicode_string(100)
        })

        # Record with nested objects
        records.append({
            "field1": self.generate_object(5),
            "field2": self.generate_object(5),
            "field3": self.generate_object(5)
        })

        # Record with large arrays
        records.append({
            "field1": self.generate_array(10000, "string"),
            "field2": self.generate_array(10000, "integer"),
            "field3": self.generate_array(10000, "float")
        })

        return records

    def save_data(self, data: List[Dict[str, Any]], output_path: Path, format: str = "json") -> None:
        """Save data to file in specified format."""
        output_path.parent.mkdir(parents=True, exist_ok=True)

        if format == "json":
            with open(output_path, 'w') as f:
                json.dump(data, f, indent=2)
        elif format == "ndjson":
            with open(output_path, 'w') as f:
                for record in data:
                    f.write(json.dumps(record) + '\n')
        elif format == "jsonl":
            with open(output_path, 'w') as f:
                for record in data:
                    f.write(json.dumps(record) + '\n')
        else:
            raise ValueError(f"Unsupported format: {format}")

    def compress_data(self, input_path: Path, output_path: Path, compression: str = "gzip") -> None:
        """Compress data file."""
        output_path.parent.mkdir(parents=True, exist_ok=True)

        with open(input_path, 'rb') as f_in:
            if compression == "gzip":
                with gzip.open(output_path, 'wb') as f_out:
                    f_out.write(f_in.read())
            elif compression == "bzip2":
                with bz2.open(output_path, 'wb') as f_out:
                    f_out.write(f_in.read())
            elif compression == "lzma":
                with lzma.open(output_path, 'wb') as f_out:
                    f_out.write(f_in.read())
            else:
                raise ValueError(f"Unsupported compression: {compression}")

    def generate_metadata(self, data: List[Dict[str, Any]], generation_params: Dict[str, Any]) -> Dict[str, Any]:
        """Generate metadata for the dataset."""
        return {
            "version": self.version,
            "generated_at": datetime.now().isoformat(),
            "generator_seed": self.seed,
            "record_count": len(data),
            "generation_params": generation_params,
            "data_hash": hashlib.sha256(json.dumps(data, sort_keys=True).encode()).hexdigest(),
            "provenance": {
                "generator": "jac_test_data_generator",
                "generator_version": "1.0.0",
                "python_version": sys.version,
                "generation_time": time.time()
            }
        }

def main():
    """Main function."""
    parser = argparse.ArgumentParser(description="Generate test data for JAC library")
    parser.add_argument("--output-dir", default=DEFAULT_OUTPUT_DIR, help="Output directory")
    parser.add_argument("--seed", type=int, default=DEFAULT_SEED, help="Random seed")
    parser.add_argument("--version", default=DEFAULT_VERSION, help="Data version")
    parser.add_argument("--format", choices=["json", "ndjson", "jsonl"], default="ndjson", help="Output format")
    parser.add_argument("--compress", choices=["gzip", "bzip2", "lzma"], help="Compression format")
    parser.add_argument("--category", choices=["unit", "integration", "performance", "stress", "conformance"],
                       default="integration", help="Test data category")
    parser.add_argument("--size", choices=["small", "medium", "large", "xlarge"],
                       default="medium", help="Data size")
    parser.add_argument("--count", type=int, help="Number of records to generate")
    parser.add_argument("--field-types", nargs="+", default=FIELD_TYPES, help="Field types to generate")

    args = parser.parse_args()

    # Set default count based on size
    if args.count is None:
        size_counts = {
            "small": 100,
            "medium": 1000,
            "large": 10000,
            "xlarge": 100000
        }
        args.count = size_counts[args.size]

    # Initialize generator
    generator = TestDataGenerator(seed=args.seed, version=args.version)

    # Generate data based on category
    if args.category == "unit":
        data = generator.generate_edge_case_data()
    elif args.category == "integration":
        data = [generator.generate_record(args.field_types) for _ in range(args.count)]
    elif args.category == "performance":
        data = generator.generate_log_data(args.count)
    elif args.category == "stress":
        data = generator.generate_compression_test_data(args.count)
    elif args.category == "conformance":
        data = generator.generate_time_series_data(args.count, datetime.now())
    else:
        data = [generator.generate_record(args.field_types) for _ in range(args.count)]

    # Generate output path
    output_dir = Path(args.output_dir) / args.category / args.size
    output_file = output_dir / f"test_data_{args.category}_{args.size}_{args.count}.{args.format}"

    # Save data
    generator.save_data(data, output_file, args.format)

    # Compress if requested
    if args.compress:
        compressed_file = output_file.with_suffix(f".{args.format}.{args.compress}")
        generator.compress_data(output_file, compressed_file, args.compress)
        print(f"Compressed data saved to: {compressed_file}")

    # Generate and save metadata
    metadata = generator.generate_metadata(data, vars(args))
    metadata_file = output_dir / f"test_data_{args.category}_{args.size}_{args.count}_metadata.json"
    with open(metadata_file, 'w') as f:
        json.dump(metadata, f, indent=2)

    print(f"Generated {len(data)} records")
    print(f"Data saved to: {output_file}")
    print(f"Metadata saved to: {metadata_file}")
    print(f"Data hash: {metadata['data_hash']}")

if __name__ == "__main__":
    main()
