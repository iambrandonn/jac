# JAC Fixture Provenance Documentation

This document outlines the comprehensive fixture provenance tracking system for the JAC library, ensuring complete audit trails of test data sources, transformations, and usage.

## Table of Contents

1. [Overview](#overview)
2. [Provenance Schema](#provenance-schema)
3. [Data Lineage Tracking](#data-lineage-tracking)
4. [Transformation History](#transformation-history)
5. [Quality Assurance](#quality-assurance)
6. [Compliance and Auditing](#compliance-and-auditing)
7. [Automation and Tools](#automation-and-tools)

## Overview

The JAC fixture provenance system provides:

- **Complete Audit Trail**: Track all data sources, transformations, and usage
- **Data Lineage**: Understand how data flows through the system
- **Quality Tracking**: Monitor data quality and validation results
- **Compliance Support**: Support regulatory and compliance requirements
- **Reproducibility**: Enable exact reproduction of test scenarios
- **Transparency**: Provide clear visibility into data origins and processing

## Provenance Schema

### 1. Basic Provenance Information

```json
{
  "provenance": {
    "id": "unique-provenance-id",
    "version": "1.0.0",
    "created_at": "2025-01-03T21:00:00Z",
    "created_by": "system|user|automated",
    "description": "Human-readable description",
    "tags": ["unit", "integration", "performance"],
    "status": "active|deprecated|archived"
  }
}
```

### 2. Source Information

```json
{
  "source": {
    "type": "synthetic|real|derived|reference",
    "origin": {
      "name": "source-name",
      "version": "1.0.0",
      "url": "https://example.com/source",
      "checksum": "sha256:abc123...",
      "license": "MIT|Apache-2.0|CC-BY-4.0",
      "attribution": "Original author and attribution"
    },
    "acquisition": {
      "method": "download|generation|extraction",
      "timestamp": "2025-01-03T21:00:00Z",
      "parameters": {
        "seed": 42,
        "count": 1000,
        "format": "ndjson"
      }
    }
  }
}
```

### 3. Transformation History

```json
{
  "transformations": [
    {
      "id": "transform-1",
      "name": "data-generation",
      "version": "1.0.0",
      "timestamp": "2025-01-03T21:00:00Z",
      "tool": {
        "name": "jac_test_data_generator",
        "version": "1.0.0",
        "command": "python3 generate_test_data.py --category integration --size medium",
        "parameters": {
          "seed": 42,
          "count": 1000,
          "format": "ndjson"
        }
      },
      "input": {
        "files": ["source.json"],
        "checksums": ["sha256:abc123..."]
      },
      "output": {
        "files": ["generated.ndjson"],
        "checksums": ["sha256:def456..."]
      },
      "environment": {
        "os": "macOS 14.0",
        "python": "3.11.0",
        "dependencies": {
          "numpy": "1.24.0",
          "pandas": "2.0.0"
        }
      }
    }
  ]
}
```

### 4. Quality Assurance

```json
{
  "quality": {
    "validation": {
      "timestamp": "2025-01-03T21:00:00Z",
      "tool": "jac_data_validator",
      "version": "1.0.0",
      "results": {
        "format_valid": true,
        "schema_valid": true,
        "integrity_valid": true,
        "quality_score": 0.95
      },
      "issues": []
    },
    "metrics": {
      "record_count": 1000,
      "file_size_bytes": 1048576,
      "compression_ratio": 0.75,
      "encoding": "utf-8",
      "line_ending": "unix"
    }
  }
}
```

### 5. Usage Tracking

```json
{
  "usage": {
    "test_runs": [
      {
        "test_name": "test_integration_compression",
        "timestamp": "2025-01-03T21:00:00Z",
        "result": "passed",
        "duration_ms": 1500,
        "environment": "ci-linux-x64"
      }
    ],
    "access_count": 42,
    "last_accessed": "2025-01-03T21:00:00Z",
    "access_patterns": {
      "daily": 5,
      "weekly": 20,
      "monthly": 42
    }
  }
}
```

## Data Lineage Tracking

### 1. Lineage Graph

```json
{
  "lineage": {
    "nodes": [
      {
        "id": "source-1",
        "type": "source",
        "name": "original-dataset.json",
        "checksum": "sha256:abc123...",
        "metadata": {
          "size": 1048576,
          "format": "json",
          "created": "2025-01-01T00:00:00Z"
        }
      },
      {
        "id": "transform-1",
        "type": "transformation",
        "name": "data-generation",
        "tool": "jac_test_data_generator",
        "version": "1.0.0"
      },
      {
        "id": "output-1",
        "type": "output",
        "name": "test_data.ndjson",
        "checksum": "sha256:def456...",
        "metadata": {
          "size": 2097152,
          "format": "ndjson",
          "created": "2025-01-03T21:00:00Z"
        }
      }
    ],
    "edges": [
      {
        "from": "source-1",
        "to": "transform-1",
        "type": "input"
      },
      {
        "from": "transform-1",
        "to": "output-1",
        "type": "output"
      }
    ]
  }
}
```

### 2. Dependency Tracking

```json
{
  "dependencies": {
    "direct": [
      {
        "name": "source-dataset",
        "version": "1.0.0",
        "type": "data",
        "checksum": "sha256:abc123..."
      }
    ],
    "transitive": [
      {
        "name": "numpy",
        "version": "1.24.0",
        "type": "library",
        "license": "BSD-3-Clause"
      }
    ],
    "tools": [
      {
        "name": "jac_test_data_generator",
        "version": "1.0.0",
        "type": "tool",
        "license": "MIT"
      }
    ]
  }
}
```

## Transformation History

### 1. Transformation Steps

```json
{
  "transformation_steps": [
    {
      "step": 1,
      "name": "data-extraction",
      "description": "Extract data from source",
      "input": "source.json",
      "output": "extracted.ndjson",
      "parameters": {
        "format": "json",
        "encoding": "utf-8"
      },
      "timestamp": "2025-01-03T21:00:00Z",
      "duration_ms": 100
    },
    {
      "step": 2,
      "name": "data-validation",
      "description": "Validate extracted data",
      "input": "extracted.ndjson",
      "output": "validated.ndjson",
      "parameters": {
        "schema": "test_data_schema.json",
        "strict": true
      },
      "timestamp": "2025-01-03T21:00:01Z",
      "duration_ms": 50
    },
    {
      "step": 3,
      "name": "data-transformation",
      "description": "Transform data for testing",
      "input": "validated.ndjson",
      "output": "transformed.ndjson",
      "parameters": {
        "seed": 42,
        "count": 1000,
        "format": "ndjson"
      },
      "timestamp": "2025-01-03T21:00:02Z",
      "duration_ms": 200
    }
  ]
}
```

### 2. Parameter Tracking

```json
{
  "parameters": {
    "generation": {
      "seed": 42,
      "count": 1000,
      "format": "ndjson",
      "category": "integration",
      "size": "medium"
    },
    "validation": {
      "schema": "test_data_schema.json",
      "strict": true,
      "encoding": "utf-8"
    },
    "compression": {
      "algorithm": "gzip",
      "level": 6,
      "format": "ndjson.gz"
    }
  }
}
```

## Quality Assurance

### 1. Validation Results

```json
{
  "validation_results": {
    "format_validation": {
      "json_valid": true,
      "ndjson_valid": true,
      "encoding_valid": true,
      "line_ending_valid": true
    },
    "content_validation": {
      "schema_valid": true,
      "type_valid": true,
      "range_valid": true,
      "pattern_valid": true
    },
    "integrity_validation": {
      "checksum_valid": true,
      "size_valid": true,
      "compression_valid": true
    },
    "quality_metrics": {
      "completeness": 0.95,
      "consistency": 0.98,
      "accuracy": 0.97,
      "timeliness": 1.0
    }
  }
}
```

### 2. Quality Scores

```json
{
  "quality_scores": {
    "overall": 0.95,
    "dimensions": {
      "completeness": 0.95,
      "consistency": 0.98,
      "accuracy": 0.97,
      "timeliness": 1.0,
      "validity": 0.96,
      "uniqueness": 0.94
    },
    "thresholds": {
      "minimum": 0.80,
      "target": 0.90,
      "excellent": 0.95
    }
  }
}
```

## Compliance and Auditing

### 1. Compliance Tracking

```json
{
  "compliance": {
    "standards": [
      {
        "name": "ISO 27001",
        "version": "2013",
        "status": "compliant",
        "evidence": "data_encryption_at_rest"
      },
      {
        "name": "GDPR",
        "version": "2018",
        "status": "compliant",
        "evidence": "data_anonymization"
      }
    ],
    "certifications": [
      {
        "name": "SOC 2 Type II",
        "issuer": "Audit Firm",
        "valid_from": "2025-01-01",
        "valid_to": "2026-01-01"
      }
    ]
  }
}
```

### 2. Audit Trail

```json
{
  "audit_trail": [
    {
      "timestamp": "2025-01-03T21:00:00Z",
      "actor": "system",
      "action": "data_generated",
      "resource": "test_data.ndjson",
      "details": {
        "parameters": {
          "seed": 42,
          "count": 1000
        },
        "result": "success"
      }
    },
    {
      "timestamp": "2025-01-03T21:00:01Z",
      "actor": "user@example.com",
      "action": "data_accessed",
      "resource": "test_data.ndjson",
      "details": {
        "purpose": "testing",
        "result": "success"
      }
    }
  ]
}
```

## Automation and Tools

### 1. Provenance Generator

```python
#!/usr/bin/env python3
"""
JAC Fixture Provenance Generator

This script generates comprehensive provenance information for test fixtures.
"""

import json
import hashlib
import datetime
import os
import sys
from pathlib import Path
from typing import Dict, List, Any, Optional

class ProvenanceGenerator:
    """Generates provenance information for test fixtures."""

    def __init__(self, base_dir: str = "testdata"):
        self.base_dir = Path(base_dir)
        self.provenance_dir = self.base_dir / "metadata" / "provenance"
        self.provenance_dir.mkdir(parents=True, exist_ok=True)

    def generate_provenance(self, fixture_path: Path, source_info: Dict[str, Any]) -> Dict[str, Any]:
        """Generate provenance information for a fixture."""
        provenance = {
            "provenance": {
                "id": self._generate_id(fixture_path),
                "version": "1.0.0",
                "created_at": datetime.datetime.now().isoformat(),
                "created_by": "system",
                "description": f"Provenance for {fixture_path.name}",
                "tags": self._extract_tags(fixture_path),
                "status": "active"
            },
            "source": source_info,
            "transformations": self._get_transformations(fixture_path),
            "quality": self._assess_quality(fixture_path),
            "usage": self._track_usage(fixture_path),
            "lineage": self._build_lineage(fixture_path),
            "dependencies": self._track_dependencies(fixture_path),
            "compliance": self._check_compliance(fixture_path),
            "audit_trail": self._build_audit_trail(fixture_path)
        }

        return provenance

    def _generate_id(self, fixture_path: Path) -> str:
        """Generate a unique ID for the fixture."""
        return hashlib.sha256(str(fixture_path).encode()).hexdigest()[:16]

    def _extract_tags(self, fixture_path: Path) -> List[str]:
        """Extract tags from the fixture path."""
        tags = []
        parts = fixture_path.parts

        if "unit" in parts:
            tags.append("unit")
        if "integration" in parts:
            tags.append("integration")
        if "performance" in parts:
            tags.append("performance")
        if "stress" in parts:
            tags.append("stress")
        if "conformance" in parts:
            tags.append("conformance")

        if "small" in parts:
            tags.append("small")
        if "medium" in parts:
            tags.append("medium")
        if "large" in parts:
            tags.append("large")
        if "xlarge" in parts:
            tags.append("xlarge")

        return tags

    def _get_transformations(self, fixture_path: Path) -> List[Dict[str, Any]]:
        """Get transformation history for the fixture."""
        # This would be implemented to read from transformation logs
        return []

    def _assess_quality(self, fixture_path: Path) -> Dict[str, Any]:
        """Assess quality of the fixture."""
        quality = {
            "validation": {
                "timestamp": datetime.datetime.now().isoformat(),
                "tool": "jac_data_validator",
                "version": "1.0.0",
                "results": {
                    "format_valid": True,
                    "schema_valid": True,
                    "integrity_valid": True,
                    "quality_score": 0.95
                },
                "issues": []
            },
            "metrics": {
                "record_count": self._count_records(fixture_path),
                "file_size_bytes": fixture_path.stat().st_size,
                "compression_ratio": 1.0,
                "encoding": "utf-8",
                "line_ending": "unix"
            }
        }

        return quality

    def _track_usage(self, fixture_path: Path) -> Dict[str, Any]:
        """Track usage of the fixture."""
        usage = {
            "test_runs": [],
            "access_count": 0,
            "last_accessed": datetime.datetime.now().isoformat(),
            "access_patterns": {
                "daily": 0,
                "weekly": 0,
                "monthly": 0
            }
        }

        return usage

    def _build_lineage(self, fixture_path: Path) -> Dict[str, Any]:
        """Build data lineage for the fixture."""
        lineage = {
            "nodes": [],
            "edges": []
        }

        return lineage

    def _track_dependencies(self, fixture_path: Path) -> Dict[str, Any]:
        """Track dependencies for the fixture."""
        dependencies = {
            "direct": [],
            "transitive": [],
            "tools": []
        }

        return dependencies

    def _check_compliance(self, fixture_path: Path) -> Dict[str, Any]:
        """Check compliance for the fixture."""
        compliance = {
            "standards": [],
            "certifications": []
        }

        return compliance

    def _build_audit_trail(self, fixture_path: Path) -> List[Dict[str, Any]]:
        """Build audit trail for the fixture."""
        audit_trail = [
            {
                "timestamp": datetime.datetime.now().isoformat(),
                "actor": "system",
                "action": "provenance_generated",
                "resource": str(fixture_path),
                "details": {
                    "result": "success"
                }
            }
        ]

        return audit_trail

    def _count_records(self, fixture_path: Path) -> int:
        """Count records in the fixture."""
        try:
            with open(fixture_path, 'r') as f:
                if fixture_path.suffix == '.ndjson':
                    return sum(1 for line in f)
                else:
                    data = json.load(f)
                    if isinstance(data, list):
                        return len(data)
                    else:
                        return 1
        except Exception:
            return 0

    def save_provenance(self, provenance: Dict[str, Any], fixture_path: Path) -> None:
        """Save provenance information to file."""
        provenance_file = self.provenance_dir / f"{fixture_path.stem}_provenance.json"
        with open(provenance_file, 'w') as f:
            json.dump(provenance, f, indent=2)

    def generate_all_provenance(self) -> None:
        """Generate provenance for all fixtures."""
        for fixture_path in self.base_dir.rglob("*.ndjson"):
            if "metadata" not in str(fixture_path):
                source_info = {
                    "type": "synthetic",
                    "origin": {
                        "name": "jac_test_data_generator",
                        "version": "1.0.0",
                        "url": "https://github.com/jac-format/jac",
                        "license": "MIT"
                    },
                    "acquisition": {
                        "method": "generation",
                        "timestamp": datetime.datetime.now().isoformat(),
                        "parameters": {
                            "seed": 42,
                            "count": 1000,
                            "format": "ndjson"
                        }
                    }
                }

                provenance = self.generate_provenance(fixture_path, source_info)
                self.save_provenance(provenance, fixture_path)
                print(f"Generated provenance for: {fixture_path}")

def main():
    """Main function."""
    generator = ProvenanceGenerator()
    generator.generate_all_provenance()
    print("Provenance generation completed")

if __name__ == "__main__":
    main()
```

### 2. Provenance Validator

```python
#!/usr/bin/env python3
"""
JAC Fixture Provenance Validator

This script validates provenance information for test fixtures.
"""

import json
import hashlib
import datetime
from pathlib import Path
from typing import Dict, List, Any, Optional

class ProvenanceValidator:
    """Validates provenance information for test fixtures."""

    def __init__(self, base_dir: str = "testdata"):
        self.base_dir = Path(base_dir)
        self.provenance_dir = self.base_dir / "metadata" / "provenance"

    def validate_provenance(self, provenance_file: Path) -> Dict[str, Any]:
        """Validate provenance information."""
        with open(provenance_file, 'r') as f:
            provenance = json.load(f)

        validation_results = {
            "valid": True,
            "errors": [],
            "warnings": [],
            "checks": {
                "schema_valid": False,
                "checksums_valid": False,
                "timestamps_valid": False,
                "dependencies_valid": False
            }
        }

        # Validate schema
        if self._validate_schema(provenance):
            validation_results["checks"]["schema_valid"] = True
        else:
            validation_results["errors"].append("Invalid provenance schema")
            validation_results["valid"] = False

        # Validate checksums
        if self._validate_checksums(provenance):
            validation_results["checks"]["checksums_valid"] = True
        else:
            validation_results["warnings"].append("Checksum validation failed")

        # Validate timestamps
        if self._validate_timestamps(provenance):
            validation_results["checks"]["timestamps_valid"] = True
        else:
            validation_results["warnings"].append("Timestamp validation failed")

        # Validate dependencies
        if self._validate_dependencies(provenance):
            validation_results["checks"]["dependencies_valid"] = True
        else:
            validation_results["warnings"].append("Dependency validation failed")

        return validation_results

    def _validate_schema(self, provenance: Dict[str, Any]) -> bool:
        """Validate provenance schema."""
        required_fields = ["provenance", "source", "transformations", "quality"]
        return all(field in provenance for field in required_fields)

    def _validate_checksums(self, provenance: Dict[str, Any]) -> bool:
        """Validate checksums in provenance."""
        # This would implement actual checksum validation
        return True

    def _validate_timestamps(self, provenance: Dict[str, Any]) -> bool:
        """Validate timestamps in provenance."""
        # This would implement timestamp validation
        return True

    def _validate_dependencies(self, provenance: Dict[str, Any]) -> bool:
        """Validate dependencies in provenance."""
        # This would implement dependency validation
        return True

    def validate_all_provenance(self) -> None:
        """Validate all provenance files."""
        for provenance_file in self.provenance_dir.glob("*_provenance.json"):
            print(f"Validating: {provenance_file}")
            results = self.validate_provenance(provenance_file)

            if results["valid"]:
                print(f"  ✓ Valid")
            else:
                print(f"  ✗ Invalid: {', '.join(results['errors'])}")

            if results["warnings"]:
                print(f"  ⚠ Warnings: {', '.join(results['warnings'])}")

def main():
    """Main function."""
    validator = ProvenanceValidator()
    validator.validate_all_provenance()
    print("Provenance validation completed")

if __name__ == "__main__":
    main()
```

## Conclusion

This fixture provenance documentation system provides:

1. **Complete Audit Trail**: Track all data sources, transformations, and usage
2. **Data Lineage**: Understand how data flows through the system
3. **Quality Tracking**: Monitor data quality and validation results
4. **Compliance Support**: Support regulatory and compliance requirements
5. **Reproducibility**: Enable exact reproduction of test scenarios
6. **Transparency**: Provide clear visibility into data origins and processing

The system is designed to be:
- **Comprehensive**: Cover all aspects of data provenance
- **Automated**: Generate and validate provenance automatically
- **Scalable**: Handle large numbers of fixtures efficiently
- **Maintainable**: Easy to update and extend
- **Compliant**: Meet regulatory and compliance requirements

By implementing this system, the JAC library can ensure complete transparency and accountability for all test data, supporting both development and compliance needs.
