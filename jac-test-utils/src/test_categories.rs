//! Test categorization utilities for JAC
//!
//! This module provides attributes and utilities for categorizing tests
//! by their performance characteristics and execution requirements.

/// Test categories for performance and execution characteristics
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestCategory {
    /// Fast unit tests (< 1 second)
    Unit,
    /// Integration tests (1-10 seconds)
    Integration,
    /// Slow tests (10+ seconds)
    Slow,
    /// Performance/benchmark tests
    Performance,
    /// Stress tests with high resource usage
    Stress,
    /// Tests that require specific hardware or environment
    Hardware,
    /// Tests that should be ignored by default
    Ignored,
}

impl TestCategory {
    /// Get the Rust test attribute for this category
    pub fn test_attribute(&self) -> &'static str {
        match self {
            TestCategory::Unit => "#[test]",
            TestCategory::Integration => "#[test]",
            TestCategory::Slow => "#[test] #[ignore]",
            TestCategory::Performance => "#[test] #[ignore]",
            TestCategory::Stress => "#[test] #[ignore]",
            TestCategory::Hardware => "#[test] #[ignore]",
            TestCategory::Ignored => "#[test] #[ignore]",
        }
    }

    /// Get the category name for documentation
    pub fn name(&self) -> &'static str {
        match self {
            TestCategory::Unit => "unit",
            TestCategory::Integration => "integration",
            TestCategory::Slow => "slow",
            TestCategory::Performance => "performance",
            TestCategory::Stress => "stress",
            TestCategory::Hardware => "hardware",
            TestCategory::Ignored => "ignored",
        }
    }

    /// Get the category description
    pub fn description(&self) -> &'static str {
        match self {
            TestCategory::Unit => "Fast unit tests that should run in CI",
            TestCategory::Integration => "Integration tests that run in CI",
            TestCategory::Slow => "Slow tests that are skipped in CI but run in nightly",
            TestCategory::Performance => "Performance tests that require dedicated time",
            TestCategory::Stress => "Stress tests that consume significant resources",
            TestCategory::Hardware => "Tests requiring specific hardware or environment",
            TestCategory::Ignored => "Tests that are disabled by default",
        }
    }
}

/// Test execution requirements
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestRequirement {
    /// No special requirements
    None,
    /// Requires network access
    Network,
    /// Requires specific hardware (e.g., specific CPU features)
    Hardware,
    /// Requires significant memory (> 1GB)
    HighMemory,
    /// Requires significant disk space (> 100MB)
    HighDisk,
    /// Requires specific operating system
    OsSpecific,
    /// Requires specific architecture
    ArchSpecific,
}

impl TestRequirement {
    /// Get the requirement name
    pub fn name(&self) -> &'static str {
        match self {
            TestRequirement::None => "none",
            TestRequirement::Network => "network",
            TestRequirement::Hardware => "hardware",
            TestRequirement::HighMemory => "high_memory",
            TestRequirement::HighDisk => "high_disk",
            TestRequirement::OsSpecific => "os_specific",
            TestRequirement::ArchSpecific => "arch_specific",
        }
    }

    /// Get the requirement description
    pub fn description(&self) -> &'static str {
        match self {
            TestRequirement::None => "No special requirements",
            TestRequirement::Network => "Requires network access",
            TestRequirement::Hardware => "Requires specific hardware",
            TestRequirement::HighMemory => "Requires significant memory",
            TestRequirement::HighDisk => "Requires significant disk space",
            TestRequirement::OsSpecific => "Requires specific operating system",
            TestRequirement::ArchSpecific => "Requires specific architecture",
        }
    }
}

/// Test metadata for categorization
#[derive(Debug, Clone)]
pub struct TestMetadata {
    pub category: TestCategory,
    pub requirement: TestRequirement,
    pub estimated_duration: Option<std::time::Duration>,
    pub memory_usage: Option<u64>, // bytes
    pub description: Option<String>,
}

impl TestMetadata {
    /// Create metadata for a unit test
    pub fn unit() -> Self {
        Self {
            category: TestCategory::Unit,
            requirement: TestRequirement::None,
            estimated_duration: Some(std::time::Duration::from_millis(100)),
            memory_usage: Some(10 * 1024 * 1024), // 10MB
            description: None,
        }
    }

    /// Create metadata for an integration test
    pub fn integration() -> Self {
        Self {
            category: TestCategory::Integration,
            requirement: TestRequirement::None,
            estimated_duration: Some(std::time::Duration::from_secs(5)),
            memory_usage: Some(100 * 1024 * 1024), // 100MB
            description: None,
        }
    }

    /// Create metadata for a slow test
    pub fn slow() -> Self {
        Self {
            category: TestCategory::Slow,
            requirement: TestRequirement::None,
            estimated_duration: Some(std::time::Duration::from_secs(30)),
            memory_usage: Some(500 * 1024 * 1024), // 500MB
            description: None,
        }
    }

    /// Create metadata for a performance test
    pub fn performance() -> Self {
        Self {
            category: TestCategory::Performance,
            requirement: TestRequirement::None,
            estimated_duration: Some(std::time::Duration::from_secs(60)),
            memory_usage: Some(1024 * 1024 * 1024), // 1GB
            description: None,
        }
    }

    /// Create metadata for a stress test
    pub fn stress() -> Self {
        Self {
            category: TestCategory::Stress,
            requirement: TestRequirement::HighMemory,
            estimated_duration: Some(std::time::Duration::from_secs(120)),
            memory_usage: Some(2 * 1024 * 1024 * 1024), // 2GB
            description: None,
        }
    }

    /// Create metadata for a hardware-specific test
    pub fn hardware() -> Self {
        Self {
            category: TestCategory::Hardware,
            requirement: TestRequirement::Hardware,
            estimated_duration: Some(std::time::Duration::from_secs(10)),
            memory_usage: Some(100 * 1024 * 1024), // 100MB
            description: None,
        }
    }

    /// Create metadata for an ignored test
    pub fn ignored() -> Self {
        Self {
            category: TestCategory::Ignored,
            requirement: TestRequirement::None,
            estimated_duration: None,
            memory_usage: None,
            description: None,
        }
    }

    /// Set a custom description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }

    /// Set custom duration
    pub fn with_duration(mut self, duration: std::time::Duration) -> Self {
        self.estimated_duration = Some(duration);
        self
    }

    /// Set custom memory usage
    pub fn with_memory_usage(mut self, memory: u64) -> Self {
        self.memory_usage = Some(memory);
        self
    }

    /// Set custom requirement
    pub fn with_requirement(mut self, requirement: TestRequirement) -> Self {
        self.requirement = requirement;
        self
    }
}

/// Test categorization registry
pub struct TestRegistry {
    tests: std::collections::HashMap<String, TestMetadata>,
}

impl TestRegistry {
    /// Create a new test registry
    pub fn new() -> Self {
        Self {
            tests: std::collections::HashMap::new(),
        }
    }

    /// Register a test with metadata
    pub fn register(&mut self, test_name: String, metadata: TestMetadata) {
        self.tests.insert(test_name, metadata);
    }

    /// Get metadata for a test
    pub fn get(&self, test_name: &str) -> Option<&TestMetadata> {
        self.tests.get(test_name)
    }

    /// Get all tests in a category
    pub fn get_tests_in_category(&self, category: TestCategory) -> Vec<(&String, &TestMetadata)> {
        self.tests
            .iter()
            .filter(|(_, metadata)| metadata.category == category)
            .collect()
    }

    /// Get all tests with a specific requirement
    pub fn get_tests_with_requirement(&self, requirement: TestRequirement) -> Vec<(&String, &TestMetadata)> {
        self.tests
            .iter()
            .filter(|(_, metadata)| metadata.requirement == requirement)
            .collect()
    }

    /// Generate a test categorization report
    pub fn generate_report(&self) -> String {
        let mut report = String::new();
        report.push_str("# Test Categorization Report\n\n");

        for category in [
            TestCategory::Unit,
            TestCategory::Integration,
            TestCategory::Slow,
            TestCategory::Performance,
            TestCategory::Stress,
            TestCategory::Hardware,
            TestCategory::Ignored,
        ] {
            let tests = self.get_tests_in_category(category);
            if !tests.is_empty() {
                report.push_str(&format!("## {} Tests ({})\n\n", category.name(), tests.len()));
                report.push_str(&format!("{}\n\n", category.description()));

                for (test_name, metadata) in tests {
                    report.push_str(&format!("- `{}`", test_name));
                    if let Some(desc) = &metadata.description {
                        report.push_str(&format!(": {}", desc));
                    }
                    if let Some(duration) = metadata.estimated_duration {
                        report.push_str(&format!(" (estimated: {:?})", duration));
                    }
                    if let Some(memory) = metadata.memory_usage {
                        report.push_str(&format!(" (memory: {}MB)", memory / 1024 / 1024));
                    }
                    if metadata.requirement != TestRequirement::None {
                        report.push_str(&format!(" (requires: {})", metadata.requirement.name()));
                    }
                    report.push('\n');
                }
                report.push('\n');
            }
        }

        report
    }
}

impl Default for TestRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_category_attributes() {
        assert_eq!(TestCategory::Unit.test_attribute(), "#[test]");
        assert_eq!(TestCategory::Slow.test_attribute(), "#[test] #[ignore]");
        assert_eq!(TestCategory::Performance.test_attribute(), "#[test] #[ignore]");
    }

    #[test]
    fn test_metadata_creation() {
        let unit_meta = TestMetadata::unit();
        assert_eq!(unit_meta.category, TestCategory::Unit);
        assert_eq!(unit_meta.requirement, TestRequirement::None);

        let stress_meta = TestMetadata::stress();
        assert_eq!(stress_meta.category, TestCategory::Stress);
        assert_eq!(stress_meta.requirement, TestRequirement::HighMemory);
    }

    #[test]
    fn test_registry() {
        let mut registry = TestRegistry::new();
        registry.register("test_example".to_string(), TestMetadata::unit());

        let metadata = registry.get("test_example").unwrap();
        assert_eq!(metadata.category, TestCategory::Unit);
    }
}
