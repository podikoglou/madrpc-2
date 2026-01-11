//! Authentication Layer for MaDRPC
//!
//! This module provides authentication support for MaDRPC nodes and orchestrators.
//! The current implementation supports API key authentication via the `X-API-Key` header.
//!
//! # Architecture
//!
//! Authentication is optional and can be configured per server:
//! - **No authentication**: Server accepts all requests (default for backward compatibility)
//! - **API key authentication**: Server requires `X-API-Key` header with a valid shared secret
//!
//! # Security Model
//!
//! - API keys are shared secrets between clients and servers
//! - Keys are validated using constant-time comparison to prevent timing attacks
//! - Failed authentication results in HTTP 401 Unauthorized
//!
//! # Example
//!
//! ```no_run
//! use madrpc_common::auth::AuthConfig;
//!
//! // Create a server with API key authentication
//! let auth = AuthConfig::with_api_key("my-secret-key-12345");
//!
//! // Create a server without authentication
//! let no_auth = AuthConfig::disabled();
//! ```

use std::fmt;

/// Authentication configuration for MaDRPC servers.
///
/// This struct defines how a server (node or orchestrator) should authenticate
/// incoming requests. Authentication is optional to maintain backward compatibility
/// and allow for different deployment scenarios.
///
/// # Variants
///
/// - **Disabled**: No authentication required (default)
/// - **ApiKey**: Requires valid API key via `X-API-Key` header
///
/// # Example
///
/// ```
/// use madrpc_common::auth::AuthConfig;
///
/// // Disable authentication (default)
/// let auth = AuthConfig::disabled();
/// assert!(!auth.requires_auth());
///
/// // Enable API key authentication
/// let auth = AuthConfig::with_api_key("secret-key");
/// assert!(auth.requires_auth());
/// assert!(auth.validate_api_key("secret-key"));
/// ```
#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// The API key if authentication is enabled, None if disabled
    api_key: Option<String>,
}

impl AuthConfig {
    /// Creates a new `AuthConfig` with API key authentication enabled.
    ///
    /// # Arguments
    ///
    /// * `api_key` - The secret API key that clients must provide
    ///
    /// # Returns
    ///
    /// A new `AuthConfig` with authentication enabled
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::auth::AuthConfig;
    ///
    /// let auth = AuthConfig::with_api_key("my-secret-key");
    /// assert!(auth.requires_auth());
    /// ```
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self {
            api_key: Some(api_key.into()),
        }
    }

    /// Creates a new `AuthConfig` with authentication disabled.
    ///
    /// This is the default configuration for backward compatibility.
    ///
    /// # Returns
    ///
    /// A new `AuthConfig` with authentication disabled
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::auth::AuthConfig;
    ///
    /// let auth = AuthConfig::disabled();
    /// assert!(!auth.requires_auth());
    /// ```
    pub fn disabled() -> Self {
        Self {
            api_key: None,
        }
    }

    /// Returns whether authentication is required.
    ///
    /// # Returns
    ///
    /// `true` if authentication is enabled, `false` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::auth::AuthConfig;
    ///
    /// let auth = AuthConfig::with_api_key("secret");
    /// assert!(auth.requires_auth());
    ///
    /// let no_auth = AuthConfig::disabled();
    /// assert!(!no_auth.requires_auth());
    /// ```
    pub fn requires_auth(&self) -> bool {
        self.api_key.is_some()
    }

    /// Validates an API key against the configured key.
    ///
    /// This method uses constant-time comparison to prevent timing attacks.
    /// If authentication is disabled, this always returns `true`.
    ///
    /// # Arguments
    ///
    /// * `provided_key` - The API key provided by the client
    ///
    /// # Returns
    ///
    /// `true` if the key is valid or authentication is disabled, `false` otherwise
    ///
    /// # Example
    ///
    /// ```
    /// use madrpc_common::auth::AuthConfig;
    ///
    /// let auth = AuthConfig::with_api_key("correct-key");
    /// assert!(auth.validate_api_key("correct-key"));
    /// assert!(!auth.validate_api_key("wrong-key"));
    ///
    /// let no_auth = AuthConfig::disabled();
    /// assert!(no_auth.validate_api_key("anything"));
    /// ```
    pub fn validate_api_key(&self, provided_key: &str) -> bool {
        match &self.api_key {
            Some(expected_key) => constant_time_eq(expected_key, provided_key),
            None => true, // No auth required
        }
    }
}

impl Default for AuthConfig {
    /// Creates a default `AuthConfig` with authentication disabled.
    ///
    /// This maintains backward compatibility by not requiring authentication
    /// unless explicitly configured.
    fn default() -> Self {
        Self::disabled()
    }
}

impl fmt::Display for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.api_key {
            Some(_key) => write!(f, "ApiKey(*****)"),
            None => write!(f, "Disabled"),
        }
    }
}

/// Performs constant-time string comparison to prevent timing attacks.
///
/// This function always iterates through the entire strings regardless of
/// where the first difference occurs, preventing attackers from using timing
/// information to guess the correct key byte-by-byte.
///
/// # Arguments
///
/// * `a` - First string to compare
/// * `b` - Second string to compare
///
/// # Returns
///
/// `true` if the strings are equal, `false` otherwise
fn constant_time_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut result = 0u8;
    for (byte_a, byte_b) in a.bytes().zip(b.bytes()) {
        result |= byte_a ^ byte_b;
    }

    result == 0
}

/// Extracts the API key from HTTP headers.
///
/// This function looks for the `X-API-Key` header in the provided header map.
///
/// # Arguments
///
/// * `header_value` - Optional string slice containing the header value
///
/// # Returns
///
/// `Some(key)` if the header is present, `None` otherwise
///
/// # Example
///
/// ```
/// use madrpc_common::auth::extract_api_key;
///
/// assert_eq!(extract_api_key(Some("my-secret-key")), Some("my-secret-key"));
/// assert_eq!(extract_api_key(None), None);
/// ```
pub fn extract_api_key(header_value: Option<&str>) -> Option<&str> {
    header_value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_config_with_api_key() {
        let auth = AuthConfig::with_api_key("test-key");
        assert!(auth.requires_auth());
        assert!(auth.validate_api_key("test-key"));
        assert!(!auth.validate_api_key("wrong-key"));
    }

    #[test]
    fn test_auth_config_disabled() {
        let auth = AuthConfig::disabled();
        assert!(!auth.requires_auth());
        assert!(auth.validate_api_key("anything"));
        assert!(auth.validate_api_key(""));
    }

    #[test]
    fn test_auth_config_default() {
        let auth = AuthConfig::default();
        assert!(!auth.requires_auth());
    }

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq("hello", "hello"));
        assert!(constant_time_eq("", ""));
        assert!(constant_time_eq("same-key-123", "same-key-123"));
    }

    #[test]
    fn test_constant_time_eq_not_equal() {
        assert!(!constant_time_eq("hello", "world"));
        assert!(!constant_time_eq("a", "b"));
        assert!(!constant_time_eq("key1", "key2"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq("short", "longer"));
        assert!(!constant_time_eq("a", ""));
        assert!(!constant_time_eq("", "b"));
    }

    #[test]
    fn test_extract_api_key() {
        assert_eq!(extract_api_key(Some("my-key")), Some("my-key"));
        assert_eq!(extract_api_key(None), None);
    }

    #[test]
    fn test_auth_config_display() {
        let auth = AuthConfig::with_api_key("secret");
        assert_eq!(format!("{}", auth), "ApiKey(*****)");

        let no_auth = AuthConfig::disabled();
        assert_eq!(format!("{}", no_auth), "Disabled");
    }
}
