use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustbinConfig {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub rate_limiting: RateLimitingConfig,
    pub limits: LimitsConfig,
    pub cleanup: CleanupConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server bind address (default: "0.0.0.0")
    pub host: String,
    /// Server port (default: 3000)
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database URL (e.g., "sqlite://rustbin.db")
    pub url: String,
    /// Maximum number of database connections (default: 5)
    pub max_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitingConfig {
    /// Requests allowed per second (default: 2)
    pub requests_per_second: u32,
    /// Burst size for rate limiting (default: 5)
    pub burst_size: u32,
    /// Interval in seconds for rate limit cleanup (default: 60)
    pub cleanup_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitsConfig {
    /// Maximum number of requests stored per bin (default: 100)
    pub max_requests_per_bin: i64,
    /// Maximum body size in bytes (default: 1048576 = 1MB)
    pub max_body_size: usize,
    /// Maximum headers size in bytes (default: 1048576 = 1MB)
    pub max_headers_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    /// How long in hours to keep inactive bins (default: 1)
    pub bin_expiry_hours: i64,
    /// Cleanup task interval in seconds (default: 60)
    pub cleanup_interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Rust log filter (default: "rustbin=info,tower_http=warn,sqlx=warn,hyper=warn")
    pub filter: String,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_requests_per_bin: 100,
            max_body_size: 1024 * 1024, // 1MB
            max_headers_size: 1024 * 1024, // 1MB
        }
    }
}

impl Default for RustbinConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 3000,
            },
            database: DatabaseConfig {
                url: "sqlite://rustbin.db".to_string(),
                max_connections: 5,
            },
            rate_limiting: RateLimitingConfig {
                requests_per_second: 2,
                burst_size: 5,
                cleanup_interval_seconds: 60,
            },
            limits: LimitsConfig {
                max_requests_per_bin: 100,
                max_body_size: 1024 * 1024, // 1MB
                max_headers_size: 1024 * 1024, // 1MB
            },
            cleanup: CleanupConfig {
                bin_expiry_hours: 1,
                cleanup_interval_seconds: 60,
            },
            logging: LoggingConfig {
                filter: "rustbin=info,tower_http=warn,sqlx=warn,hyper=warn".to_string(),
            },
        }
    }
}

impl RustbinConfig {
    /// Load configuration from a TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let config: RustbinConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from a TOML file, falling back to defaults if file doesn't exist
    pub fn from_file_or_default<P: AsRef<Path>>(path: P) -> Self {
        match Self::from_file(path.as_ref()) {
            Ok(config) => {
                tracing::info!("Loaded configuration from {}", path.as_ref().display());
                config
            }
            Err(err) => {
                tracing::warn!(
                    "Failed to load config from {}: {}. Using defaults.",
                    path.as_ref().display(),
                    err
                );
                Self::default()
            }
        }
    }

    /// Save the current configuration to a TOML file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn std::error::Error>> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Create a default configuration file if it doesn't exist
    pub fn create_default_config_if_missing<P: AsRef<Path>>(path: P) -> Result<(), Box<dyn std::error::Error>> {
        if !path.as_ref().exists() {
            let default_config = Self::default();
            default_config.save_to_file(&path)?;
            tracing::info!("Created default configuration at {}", path.as_ref().display());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::NamedTempFile;

    #[test]
    fn test_default_config() {
        let config = RustbinConfig::default();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 3000);
        assert_eq!(config.database.url, "sqlite://rustbin.db");
        assert_eq!(config.database.max_connections, 5);
        assert_eq!(config.rate_limiting.requests_per_second, 2);
        assert_eq!(config.rate_limiting.burst_size, 5);
        assert_eq!(config.limits.max_requests_per_bin, 100);
        assert_eq!(config.limits.max_body_size, 1024 * 1024);
        assert_eq!(config.limits.max_headers_size, 1024 * 1024);
        assert_eq!(config.cleanup.bin_expiry_hours, 1);
        assert_eq!(config.cleanup.cleanup_interval_seconds, 60);
    }

    #[test]
    fn test_save_and_load_config() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_path = temp_file.path();

        let original_config = RustbinConfig::default();
        original_config.save_to_file(config_path).unwrap();

        let loaded_config = RustbinConfig::from_file(config_path).unwrap();
        
        assert_eq!(original_config.server.host, loaded_config.server.host);
        assert_eq!(original_config.server.port, loaded_config.server.port);
        assert_eq!(original_config.database.url, loaded_config.database.url);
    }

    #[test]
    fn test_from_file_or_default_with_missing_file() {
        let config = RustbinConfig::from_file_or_default("nonexistent.toml");
        assert_eq!(config.server.port, 3000); // Should use defaults
    }
}