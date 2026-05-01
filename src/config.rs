//! Environment- and file-driven configuration for [`crate::ServiceBootstrap`].

use std::path::Path;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Logging output format.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Pretty, human-readable lines.
    #[default]
    Pretty,
    /// One JSON object per line — for log shippers.
    Json,
}

/// Backend store selection for [`RateLimitConfig`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RateLimitKind {
    /// No rate limiting.
    #[default]
    None,
    /// In-memory limiter (single-shard `HashMap`).
    Memory {
        /// Max requests per window.
        limit: u32,
        /// Window in seconds.
        window_secs: u64,
    },
}

/// Rate-limiting configuration for [`BootstrapConfig`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RateLimitConfig {
    /// Backend store + capacity settings.
    #[serde(flatten)]
    pub kind: RateLimitKind,
}

/// Structured CORS configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CorsConfig {
    /// Allowed origins (exact match). Use `["*"]` to allow any origin.
    pub allowed_origins: Vec<String>,
    /// Allowed methods.
    pub allowed_methods: Vec<String>,
    /// Allowed request headers.
    pub allowed_headers: Vec<String>,
    /// Headers exposed to the browser.
    pub expose_headers: Vec<String>,
    /// Whether to allow credentials.
    pub allow_credentials: bool,
    /// Preflight cache `max-age` in seconds.
    pub max_age_secs: Option<u64>,
}

impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            allowed_origins: Vec::new(),
            allowed_methods: vec![
                "GET".into(),
                "POST".into(),
                "PUT".into(),
                "DELETE".into(),
                "PATCH".into(),
            ],
            allowed_headers: vec![
                "content-type".into(),
                "authorization".into(),
                "x-request-id".into(),
            ],
            expose_headers: Vec::new(),
            allow_credentials: false,
            max_age_secs: None,
        }
    }
}

/// Layered configuration consumed by [`crate::ServiceBootstrap::from_config`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BootstrapConfig {
    /// Listener bind address. Default: `0.0.0.0:8080`.
    pub bind_addr: String,
    /// `tracing` env-filter directive. Default: `info`.
    pub log_level: String,
    /// Log output format. Default: `pretty`.
    pub log_format: LogFormat,
    /// Service version reported by the liveness endpoint.
    pub version: Option<String>,
    /// Health endpoint base path. Default: `/health`.
    pub health_path: String,
    /// OpenTelemetry collector endpoint. Honors `OTEL_EXPORTER_OTLP_ENDPOINT`.
    pub otel_endpoint: Option<String>,
    /// Postgres URL. Honors `DATABASE_URL`.
    pub database_url: Option<String>,
    /// Postgres pool max connections.
    pub pool_max_connections: Option<u32>,
    /// Rate-limit backend selection.
    pub rate_limit: RateLimitConfig,
    /// Maximum request body size in bytes. Default: 2 MiB.
    pub body_limit_bytes: usize,
    /// Graceful shutdown deadline in seconds. Default: 30.
    pub shutdown_timeout_secs: u64,
    /// CORS policy.
    pub cors: CorsConfig,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8080".into(),
            log_level: "info".into(),
            log_format: LogFormat::default(),
            version: None,
            health_path: "/health".into(),
            otel_endpoint: None,
            database_url: None,
            pool_max_connections: None,
            rate_limit: RateLimitConfig::default(),
            body_limit_bytes: 2 * 1024 * 1024,
            shutdown_timeout_secs: 30,
            cors: CorsConfig::default(),
        }
    }
}

impl BootstrapConfig {
    /// Load from environment variables only.
    ///
    /// # Errors
    ///
    /// Returns an error if required config values are missing or invalid.
    pub fn from_env() -> Result<Self> {
        Self::figment(None::<&Path>)
            .extract()
            .map_err(|e| map_err(&e))
    }

    /// Load from a TOML file, with environment variables overriding any values present.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be parsed or required values are invalid.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Self::figment(Some(path.as_ref()))
            .extract()
            .map_err(|e| map_err(&e))
    }

    fn figment(path: Option<&Path>) -> Figment {
        let mut fig = Figment::from(Serialized::defaults(Self::default()));
        if let Some(p) = path {
            fig = fig.merge(Toml::file(p));
        }
        fig.merge(Env::prefixed("SOCLE_").split("__"))
            .merge(
                Env::raw()
                    .only(&["DATABASE_URL"])
                    .map(|_| "database_url".into()),
            )
            .merge(
                Env::raw()
                    .only(&["OTEL_EXPORTER_OTLP_ENDPOINT"])
                    .map(|_| "otel_endpoint".into()),
            )
    }

    /// Validate cross-field invariants. Returns the config back on success.
    ///
    /// # Errors
    ///
    /// Returns an error if any config field fails validation.
    pub fn validate(self) -> Result<Self> {
        if let RateLimitKind::Memory { limit, window_secs } = self.rate_limit.kind {
            if limit == 0 {
                return Err(Error::Config("rate_limit.limit must be > 0".into()));
            }
            if window_secs == 0 {
                return Err(Error::Config("rate_limit.window_secs must be > 0".into()));
            }
        }
        Ok(self)
    }
}

fn map_err(e: &figment::Error) -> Error {
    Error::Config(format!("config: {e}"))
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::sync::Mutex;

    // Env-var tests must run serially to avoid cross-test pollution.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_are_sensible() {
        let cfg = BootstrapConfig::default();
        assert_eq!(cfg.bind_addr, "0.0.0.0:8080");
        assert_eq!(cfg.health_path, "/health");
        assert_eq!(cfg.body_limit_bytes, 2 * 1024 * 1024);
        assert_eq!(cfg.shutdown_timeout_secs, 30);
        assert!(matches!(cfg.rate_limit.kind, RateLimitKind::None));
        assert!(matches!(cfg.log_format, LogFormat::Pretty));
    }

    #[test]
    fn from_env_returns_defaults_when_no_extra_env_set() {
        let _g = ENV_LOCK.lock().unwrap();
        let cfg = BootstrapConfig::from_env().unwrap();
        assert_eq!(cfg.bind_addr, "0.0.0.0:8080");
    }

    #[test]
    fn from_env_reads_socle_prefixed_vars() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serialises all env-var mutations in this module.
        unsafe { std::env::set_var("SOCLE_BIND_ADDR", "127.0.0.1:9999") };
        let cfg = BootstrapConfig::from_env().unwrap();
        unsafe { std::env::remove_var("SOCLE_BIND_ADDR") };
        assert_eq!(cfg.bind_addr, "127.0.0.1:9999");
    }

    #[test]
    fn from_env_reads_database_url() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serialises all env-var mutations in this module.
        unsafe { std::env::set_var("DATABASE_URL", "postgres://test/db") };
        let cfg = BootstrapConfig::from_env().unwrap();
        unsafe { std::env::remove_var("DATABASE_URL") };
        assert_eq!(cfg.database_url.as_deref(), Some("postgres://test/db"));
    }

    #[test]
    fn from_env_reads_otel_endpoint() {
        let _g = ENV_LOCK.lock().unwrap();
        // SAFETY: ENV_LOCK serialises all env-var mutations in this module.
        unsafe { std::env::set_var("OTEL_EXPORTER_OTLP_ENDPOINT", "http://otel:4317") };
        let cfg = BootstrapConfig::from_env().unwrap();
        unsafe { std::env::remove_var("OTEL_EXPORTER_OTLP_ENDPOINT") };
        assert_eq!(cfg.otel_endpoint.as_deref(), Some("http://otel:4317"));
    }

    #[test]
    fn load_reads_toml_file() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"bind_addr = "0.0.0.0:1234""#).unwrap();
        let cfg = BootstrapConfig::load(f.path()).unwrap();
        assert_eq!(cfg.bind_addr, "0.0.0.0:1234");
    }

    #[test]
    fn load_env_overrides_toml() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"bind_addr = "0.0.0.0:1234""#).unwrap();
        // SAFETY: ENV_LOCK serialises all env-var mutations in this module.
        unsafe { std::env::set_var("SOCLE_BIND_ADDR", "0.0.0.0:5678") };
        let cfg = BootstrapConfig::load(f.path()).unwrap();
        unsafe { std::env::remove_var("SOCLE_BIND_ADDR") };
        assert_eq!(cfg.bind_addr, "0.0.0.0:5678");
    }

    #[test]
    fn validate_passes_for_defaults() {
        assert!(BootstrapConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_rejects_zero_rate_limit() {
        let cfg = BootstrapConfig {
            rate_limit: RateLimitConfig {
                kind: RateLimitKind::Memory {
                    limit: 0,
                    window_secs: 60,
                },
            },
            ..Default::default()
        };
        assert!(matches!(cfg.validate(), Err(Error::Config(_))));
    }

    #[test]
    fn validate_rejects_zero_window_secs() {
        let cfg = BootstrapConfig {
            rate_limit: RateLimitConfig {
                kind: RateLimitKind::Memory {
                    limit: 10,
                    window_secs: 0,
                },
            },
            ..Default::default()
        };
        assert!(matches!(cfg.validate(), Err(Error::Config(_))));
    }

    #[test]
    fn cors_config_default_has_standard_methods() {
        let cors = CorsConfig::default();
        assert!(cors.allowed_methods.contains(&"GET".to_string()));
        assert!(cors.allowed_methods.contains(&"POST".to_string()));
        assert!(!cors.allow_credentials);
        assert!(cors.max_age_secs.is_none());
    }

    #[test]
    fn rate_limit_kind_memory_roundtrips_serde() {
        let kind = RateLimitKind::Memory {
            limit: 100,
            window_secs: 60,
        };
        let json = serde_json::to_string(&kind).unwrap();
        let back: RateLimitKind = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            RateLimitKind::Memory {
                limit: 100,
                window_secs: 60
            }
        ));
    }

    #[test]
    fn rate_limit_kind_none_roundtrips_serde() {
        let kind = RateLimitKind::None;
        let json = serde_json::to_string(&kind).unwrap();
        let back: RateLimitKind = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, RateLimitKind::None));
    }

    #[test]
    fn log_format_serde() {
        assert_eq!(
            serde_json::to_string(&LogFormat::Json).unwrap(),
            r#""json""#
        );
        assert_eq!(
            serde_json::to_string(&LogFormat::Pretty).unwrap(),
            r#""pretty""#
        );
    }

    #[test]
    fn bootstrap_config_version_field() {
        let mut cfg = BootstrapConfig::default();
        assert!(cfg.version.is_none());
        cfg.version = Some("1.2.3".into());
        assert_eq!(cfg.version.as_deref(), Some("1.2.3"));
    }
}
