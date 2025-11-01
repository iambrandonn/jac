//! Plugin system for custom wrapper implementations
//!
//! This module provides a trait-based plugin system that allows users to implement
//! custom wrapper logic for specialized JSON preprocessing needs.
//!
//! # Example
//!
//! ```rust,ignore
//! use jac_io::wrapper::plugin::{WrapperPlugin, WrapperPluginMetadata};
//! use serde_json::{Map, Value};
//!
//! struct MyCustomWrapper;
//!
//! impl WrapperPlugin for MyCustomWrapper {
//!     fn name(&self) -> &str {
//!         "my-custom-wrapper"
//!     }
//!
//!     fn process(
//!         &self,
//!         input: Box<dyn Read + Send>,
//!         config: &Value,
//!         limits: &WrapperLimits,
//!     ) -> Result<Box<dyn Iterator<Item = Map<String, Value>> + Send>, WrapperError> {
//!         // Custom processing logic
//!         todo!()
//!     }
//! }
//! ```

use super::WrapperError;
use crate::WrapperLimits;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, RwLock};

/// Metadata for a wrapper plugin
#[derive(Debug, Clone)]
pub struct WrapperPluginMetadata {
    /// Plugin name (must be unique)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Plugin version
    pub version: String,
    /// Author information
    pub author: Option<String>,
}

/// Trait for implementing custom wrapper plugins
///
/// Plugins provide custom JSON preprocessing logic that can be registered
/// and invoked through the wrapper system.
pub trait WrapperPlugin: Send + Sync {
    /// Returns the unique name of this plugin
    fn name(&self) -> &str;

    /// Returns metadata about this plugin
    fn metadata(&self) -> WrapperPluginMetadata {
        WrapperPluginMetadata {
            name: self.name().to_string(),
            description: "Custom wrapper plugin".to_string(),
            version: "1.0.0".to_string(),
            author: None,
        }
    }

    /// Process input and return an iterator of records
    ///
    /// # Arguments
    ///
    /// * `input` - The JSON input to process
    /// * `config` - Plugin-specific configuration (JSON value)
    /// * `limits` - Wrapper limits to enforce
    ///
    /// # Returns
    ///
    /// An iterator that yields Results of JSON objects (records)
    fn process(
        &self,
        input: Box<dyn Read + Send>,
        config: &Value,
        limits: &WrapperLimits,
    ) -> Result<Box<dyn Iterator<Item = Result<Map<String, Value>, WrapperError>> + Send>, WrapperError>;

    /// Optionally provide schema information for the output records
    ///
    /// This can be used to optimize encoding by providing hints about
    /// field types, cardinalities, and other characteristics.
    fn schema_hints(&self, _config: &Value) -> Option<SchemaHints> {
        None
    }

    /// Validate plugin configuration before processing
    ///
    /// Returns an error if the configuration is invalid
    fn validate_config(&self, _config: &Value) -> Result<(), WrapperError> {
        Ok(())
    }
}

/// Schema hints that can be provided by wrappers to optimize encoding
#[derive(Debug, Clone)]
pub struct SchemaHints {
    /// Expected fields and their characteristics
    pub fields: Vec<FieldHint>,
    /// Expected number of records (if known)
    pub estimated_record_count: Option<usize>,
    /// Whether all records have the same schema
    pub uniform_schema: bool,
}

/// Information about a specific field
#[derive(Debug, Clone)]
pub struct FieldHint {
    /// Field name
    pub name: String,
    /// Expected type (if uniform)
    pub expected_type: Option<FieldType>,
    /// Estimated cardinality (number of distinct values)
    pub estimated_cardinality: Option<usize>,
    /// Whether this field is always present
    pub always_present: bool,
}

/// Expected field type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    /// Null values only
    Null,
    /// Boolean values
    Bool,
    /// Integer values
    Int,
    /// Decimal/floating point values
    Decimal,
    /// String values
    String,
    /// Nested object
    Object,
    /// Array
    Array,
}

/// Global registry for wrapper plugins
pub struct WrapperPluginRegistry {
    plugins: RwLock<HashMap<String, Arc<dyn WrapperPlugin>>>,
}

impl WrapperPluginRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            plugins: RwLock::new(HashMap::new()),
        }
    }

    /// Register a plugin
    ///
    /// # Errors
    ///
    /// Returns an error if a plugin with the same name is already registered
    pub fn register(&self, plugin: Arc<dyn WrapperPlugin>) -> Result<(), WrapperError> {
        let name = plugin.name().to_string();
        let mut plugins = self.plugins.write().unwrap();

        if plugins.contains_key(&name) {
            return Err(WrapperError::PluginAlreadyRegistered { name });
        }

        plugins.insert(name, plugin);
        Ok(())
    }

    /// Unregister a plugin by name
    pub fn unregister(&self, name: &str) -> Option<Arc<dyn WrapperPlugin>> {
        let mut plugins = self.plugins.write().unwrap();
        plugins.remove(name)
    }

    /// Get a plugin by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn WrapperPlugin>> {
        let plugins = self.plugins.read().unwrap();
        plugins.get(name).cloned()
    }

    /// List all registered plugins
    pub fn list(&self) -> Vec<WrapperPluginMetadata> {
        let plugins = self.plugins.read().unwrap();
        plugins
            .values()
            .map(|p| p.metadata())
            .collect()
    }

    /// Get the global singleton instance
    pub fn global() -> &'static Self {
        static INSTANCE: std::sync::OnceLock<WrapperPluginRegistry> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(|| WrapperPluginRegistry::new())
    }
}

impl Default for WrapperPluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPlugin {
        name: String,
    }

    impl WrapperPlugin for TestPlugin {
        fn name(&self) -> &str {
            &self.name
        }

        fn process(
            &self,
            _input: Box<dyn Read + Send>,
            _config: &Value,
            _limits: &WrapperLimits,
        ) -> Result<Box<dyn Iterator<Item = Result<Map<String, Value>, WrapperError>> + Send>, WrapperError> {
            Ok(Box::new(std::iter::empty()))
        }
    }

    #[test]
    fn test_registry_register_and_get() {
        let registry = WrapperPluginRegistry::new();
        let plugin = Arc::new(TestPlugin {
            name: "test-plugin".to_string(),
        });

        registry.register(plugin.clone()).unwrap();
        let retrieved = registry.get("test-plugin").unwrap();
        assert_eq!(retrieved.name(), "test-plugin");
    }

    #[test]
    fn test_registry_duplicate_registration() {
        let registry = WrapperPluginRegistry::new();
        let plugin1 = Arc::new(TestPlugin {
            name: "test-plugin".to_string(),
        });
        let plugin2 = Arc::new(TestPlugin {
            name: "test-plugin".to_string(),
        });

        registry.register(plugin1).unwrap();
        let result = registry.register(plugin2);
        assert!(result.is_err());
    }

    #[test]
    fn test_registry_unregister() {
        let registry = WrapperPluginRegistry::new();
        let plugin = Arc::new(TestPlugin {
            name: "test-plugin".to_string(),
        });

        registry.register(plugin).unwrap();
        let removed = registry.unregister("test-plugin");
        assert!(removed.is_some());
        assert!(registry.get("test-plugin").is_none());
    }

    #[test]
    fn test_registry_list() {
        let registry = WrapperPluginRegistry::new();
        let plugin1 = Arc::new(TestPlugin {
            name: "plugin1".to_string(),
        });
        let plugin2 = Arc::new(TestPlugin {
            name: "plugin2".to_string(),
        });

        registry.register(plugin1).unwrap();
        registry.register(plugin2).unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);
    }
}
