//! Resource limits for JavaScript execution.
//!
//! This module provides configuration for limiting JavaScript execution resources
//! to prevent runaway code from consuming excessive CPU time or memory.

use std::time::Duration;

/// Resource limits for JavaScript execution.
///
/// These limits prevent malicious or buggy JavaScript code from consuming
/// excessive resources on the server.
///
/// # Fields
///
/// - `execution_timeout` - Maximum time for JavaScript execution (default: 30 seconds)
///
/// # Memory Limiting
///
/// Memory limiting is not currently supported by Boa JavaScript engine.
/// This is a known limitation that may be addressed in future versions.
/// For now, only CPU time limits are enforced.
///
/// # Example
///
/// ```
/// use madrpc_server::ResourceLimits;
/// use std::time::Duration;
///
/// // Create limits with 5 second timeout
/// let limits = ResourceLimits::new()
///     .with_execution_timeout(Duration::from_secs(5));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceLimits {
    /// Maximum time allowed for JavaScript execution before timeout
    pub execution_timeout: Duration,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            // Default to 30 seconds - generous enough for legitimate operations
            // while preventing indefinite hangs
            execution_timeout: Duration::from_secs(30),
        }
    }
}

impl ResourceLimits {
    /// Creates a new ResourceLimits with default values.
    ///
    /// # Returns
    ///
    /// A ResourceLimits instance with default timeout of 30 seconds.
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_server::ResourceLimits;
    ///
    /// let limits = ResourceLimits::new();
    /// assert_eq!(limits.execution_timeout.as_secs(), 30);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the maximum execution timeout.
    ///
    /// # Parameters
    ///
    /// * `timeout` - Maximum duration for JavaScript execution
    ///
    /// # Returns
    ///
    /// `Self` for builder pattern chaining.
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_server::ResourceLimits;
    /// use std::time::Duration;
    ///
    /// let limits = ResourceLimits::new()
    ///     .with_execution_timeout(Duration::from_secs(10));
    /// ```
    pub fn with_execution_timeout(mut self, timeout: Duration) -> Self {
        self.execution_timeout = timeout;
        self
    }

    /// Validates the resource limits configuration.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the configuration is valid, `Err` with a description otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Execution timeout is zero
    /// - Execution timeout is excessively long (> 1 hour)
    pub fn validate(&self) -> Result<(), String> {
        if self.execution_timeout.is_zero() {
            return Err("execution timeout must be greater than zero".to_string());
        }

        if self.execution_timeout.as_secs() > 3600 {
            return Err(format!(
                "execution timeout must be <= 1 hour (got {} seconds)",
                self.execution_timeout.as_secs()
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_resource_limits() {
        let limits = ResourceLimits::default();
        assert_eq!(limits.execution_timeout.as_secs(), 30);
    }

    #[test]
    fn test_new_resource_limits() {
        let limits = ResourceLimits::new();
        assert_eq!(limits.execution_timeout.as_secs(), 30);
    }

    #[test]
    fn test_with_execution_timeout() {
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(10));

        assert_eq!(limits.execution_timeout.as_secs(), 10);
    }

    #[test]
    fn test_with_execution_timeout_millis() {
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_millis(5500));

        assert_eq!(limits.execution_timeout.as_millis(), 5500);
    }

    #[test]
    fn test_validate_default() {
        let limits = ResourceLimits::default();
        assert!(limits.validate().is_ok());
    }

    #[test]
    fn test_validate_custom_timeout() {
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(60));
        assert!(limits.validate().is_ok());
    }

    #[test]
    fn test_validate_zero_timeout_fails() {
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::ZERO);
        assert!(limits.validate().is_err());
        let err = limits.validate().unwrap_err();
        assert!(err.contains("greater than zero"));
    }

    #[test]
    fn test_validate_excessive_timeout_fails() {
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(7200)); // 2 hours
        assert!(limits.validate().is_err());
        let err = limits.validate().unwrap_err();
        assert!(err.contains("1 hour"));
    }

    #[test]
    fn test_builder_pattern_chaining() {
        let limits = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(15));

        assert_eq!(limits.execution_timeout.as_secs(), 15);
    }

    #[test]
    fn test_equality() {
        let limits1 = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(10));
        let limits2 = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(10));
        let limits3 = ResourceLimits::new()
            .with_execution_timeout(Duration::from_secs(20));

        assert_eq!(limits1, limits2);
        assert_ne!(limits1, limits3);
    }
}
