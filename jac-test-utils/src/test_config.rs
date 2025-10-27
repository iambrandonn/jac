//! Test configuration and categorization for JAC tests
//!
//! This module provides a centralized configuration for test categorization
//! and execution requirements across the JAC project.

use crate::test_categories::{TestCategory, TestMetadata, TestRegistry};

/// Create the default test registry with all categorized tests
pub fn create_test_registry() -> TestRegistry {
    let mut registry = TestRegistry::new();

    // Unit tests (fast, run in CI)
    registry.register(
        "test_endianness_compatibility".to_string(),
        TestMetadata::unit(),
    );
    registry.register(
        "test_version_compatibility".to_string(),
        TestMetadata::unit(),
    );
    registry.register(
        "test_type_tag_compatibility".to_string(),
        TestMetadata::unit(),
    );
    registry.register(
        "test_limits_compatibility".to_string(),
        TestMetadata::unit(),
    );
    registry.register(
        "test_file_header_cross_platform".to_string(),
        TestMetadata::unit(),
    );
    registry.register(
        "test_block_header_cross_platform".to_string(),
        TestMetadata::unit(),
    );
    registry.register(
        "test_compression_codec_compatibility".to_string(),
        TestMetadata::unit(),
    );

    // Integration tests (medium speed, run in CI)
    registry.register(
        "test_spec_conformance_cross_platform".to_string(),
        TestMetadata::integration(),
    );
    registry.register(
        "test_parallel_writers_basic".to_string(),
        TestMetadata::integration(),
    );
    registry.register(
        "test_parallel_readers_basic".to_string(),
        TestMetadata::integration(),
    );
    registry.register(
        "test_projection_concurrency".to_string(),
        TestMetadata::integration(),
    );
    registry.register(
        "test_deterministic_output".to_string(),
        TestMetadata::integration(),
    );

    // Slow tests (skip in CI, run in nightly)
    registry.register(
        "million_record_roundtrip_and_projection".to_string(),
        TestMetadata::slow()
            .with_description(
                "Tests with 1 million records to verify multi-block handling".to_string(),
            )
            .with_duration(std::time::Duration::from_secs(30))
            .with_memory_usage(500 * 1024 * 1024), // 500MB
    );

    // Stress tests (high resource usage, skip in CI)
    registry.register(
        "test_high_contention".to_string(),
        TestMetadata::stress()
            .with_description("High-contention concurrency stress test with 8+ threads".to_string())
            .with_duration(std::time::Duration::from_secs(60))
            .with_memory_usage(1024 * 1024 * 1024), // 1GB
    );

    // Performance tests (benchmark-like, skip in CI)
    registry.register(
        "test_large_synthetic_block".to_string(),
        TestMetadata::performance()
            .with_description("Tests with large synthetic blocks (>100k records)".to_string())
            .with_duration(std::time::Duration::from_secs(45))
            .with_memory_usage(800 * 1024 * 1024), // 800MB
    );

    // Hardware-specific tests (if any)
    // registry.register("test_simd_optimizations".to_string(), TestMetadata::hardware());

    registry
}

/// Get test execution configuration based on environment
pub fn get_test_config() -> TestConfig {
    let ci = std::env::var("CI").is_ok();
    let nightly = std::env::var("NIGHTLY").is_ok();
    let stress = std::env::var("STRESS_TESTS").is_ok();
    let performance = std::env::var("PERFORMANCE_TESTS").is_ok();

    TestConfig {
        run_unit_tests: true,
        run_integration_tests: true,
        run_slow_tests: nightly || stress,
        run_stress_tests: stress,
        run_performance_tests: performance,
        run_hardware_tests: nightly,
        run_ignored_tests: false,
        max_test_duration: if ci {
            Some(std::time::Duration::from_secs(300)) // 5 minutes in CI
        } else {
            None
        },
        max_memory_usage: if ci {
            Some(1024 * 1024 * 1024) // 1GB in CI
        } else {
            None
        },
    }
}

/// Test execution configuration
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub run_unit_tests: bool,
    pub run_integration_tests: bool,
    pub run_slow_tests: bool,
    pub run_stress_tests: bool,
    pub run_performance_tests: bool,
    pub run_hardware_tests: bool,
    pub run_ignored_tests: bool,
    pub max_test_duration: Option<std::time::Duration>,
    pub max_memory_usage: Option<u64>,
}

impl TestConfig {
    /// Check if a test category should be run
    pub fn should_run_category(&self, category: TestCategory) -> bool {
        match category {
            TestCategory::Unit => self.run_unit_tests,
            TestCategory::Integration => self.run_integration_tests,
            TestCategory::Slow => self.run_slow_tests,
            TestCategory::Performance => self.run_performance_tests,
            TestCategory::Stress => self.run_stress_tests,
            TestCategory::Hardware => self.run_hardware_tests,
            TestCategory::Ignored => self.run_ignored_tests,
        }
    }

    /// Check if a test should be run based on its metadata
    pub fn should_run_test(&self, metadata: &TestMetadata) -> bool {
        if !self.should_run_category(metadata.category) {
            return false;
        }

        // Check duration limits
        if let Some(max_duration) = self.max_test_duration {
            if let Some(test_duration) = metadata.estimated_duration {
                if test_duration > max_duration {
                    return false;
                }
            }
        }

        // Check memory limits
        if let Some(max_memory) = self.max_memory_usage {
            if let Some(test_memory) = metadata.memory_usage {
                if test_memory > max_memory {
                    return false;
                }
            }
        }

        true
    }
}

/// Generate test execution report
pub fn generate_execution_report() -> String {
    let registry = create_test_registry();
    let config = get_test_config();

    let mut report = String::new();
    report.push_str("# Test Execution Report\n\n");
    report.push_str(&format!(
        "Environment: {}\n",
        if std::env::var("CI").is_ok() {
            "CI"
        } else {
            "Local"
        }
    ));
    report.push_str(&format!("Nightly: {}\n", std::env::var("NIGHTLY").is_ok()));
    report.push_str(&format!(
        "Stress Tests: {}\n",
        std::env::var("STRESS_TESTS").is_ok()
    ));
    report.push_str(&format!(
        "Performance Tests: {}\n\n",
        std::env::var("PERFORMANCE_TESTS").is_ok()
    ));

    for category in [
        TestCategory::Unit,
        TestCategory::Integration,
        TestCategory::Slow,
        TestCategory::Performance,
        TestCategory::Stress,
        TestCategory::Hardware,
        TestCategory::Ignored,
    ] {
        let tests = registry.get_tests_in_category(category);
        if !tests.is_empty() {
            let run_count = tests
                .iter()
                .filter(|(_, metadata)| config.should_run_test(metadata))
                .count();

            report.push_str(&format!(
                "## {} Tests: {}/{} will run\n\n",
                category.name(),
                run_count,
                tests.len()
            ));

            if run_count < tests.len() {
                report.push_str("Skipped tests:\n");
                for (test_name, metadata) in tests {
                    if !config.should_run_test(metadata) {
                        report.push_str(&format!("- `{}`", test_name));
                        if let Some(desc) = &metadata.description {
                            report.push_str(&format!(": {}", desc));
                        }
                        report.push('\n');
                    }
                }
                report.push('\n');
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = create_test_registry();
        assert!(registry.get("test_endianness_compatibility").is_some());
        assert!(registry
            .get("million_record_roundtrip_and_projection")
            .is_some());
    }

    #[test]
    fn test_config_creation() {
        let config = get_test_config();
        assert!(config.run_unit_tests);
        assert!(config.run_integration_tests);
    }

    #[test]
    fn test_category_filtering() {
        let config = get_test_config();
        let registry = create_test_registry();

        let unit_tests = registry.get_tests_in_category(TestCategory::Unit);
        assert!(!unit_tests.is_empty());

        for (_, metadata) in unit_tests {
            assert!(config.should_run_test(metadata));
        }
    }
}
