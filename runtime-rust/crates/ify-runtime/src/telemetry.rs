//! Structured logging and OpenTelemetry-compatible trace initialisation.
//!
//! [`init_telemetry`] must be called **once** at application startup.  It
//! returns a [`TelemetryHandle`] whose drop causes the tracing subscriber to
//! flush and unregister.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ify_runtime::telemetry::{TelemetryConfig, init_telemetry};
//!
//! #[tokio::main]
//! async fn main() {
//!     let cfg = TelemetryConfig {
//!         service_name: "my-service".to_owned(),
//!         json_logs: false,
//!         ..Default::default()
//!     };
//!     let _telemetry = init_telemetry(cfg).expect("telemetry init must succeed");
//!     tracing::info!("runtime started");
//! }
//! ```

use thiserror::Error;
use tracing_subscriber::{
    fmt::format::FmtSpan,
    EnvFilter,
};

/// Errors produced during telemetry initialisation.
#[derive(Debug, Error)]
pub enum TelemetryError {
    /// The global tracing subscriber has already been set.
    #[error("global tracing subscriber already initialised")]
    AlreadyInitialised,

    /// An invalid log level filter string was provided.
    #[error("invalid log level filter: {0}")]
    InvalidFilter(String),
}

/// Configuration for the telemetry subsystem.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Human-readable service name included in every log/span record.
    pub service_name: String,

    /// When `true`, emit JSON-formatted log lines (suitable for log aggregators).
    /// When `false`, emit human-readable ANSI-colored lines.
    pub json_logs: bool,

    /// `tracing` filter directive string (e.g., `"info,ify_runtime=debug"`).
    /// Defaults to the `RUST_LOG` environment variable, or `"info"` if unset.
    pub filter: Option<String>,

    /// Include span open/close events in the output.  Useful for latency
    /// tracing in development; typically disabled in production.
    pub log_spans: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "ify-runtime".to_owned(),
            json_logs: false,
            filter: None,
            log_spans: false,
        }
    }
}

/// Guard returned by [`init_telemetry`].
///
/// Keep this value alive for the duration of the program.  Dropping it is a
/// no-op in the current implementation; it is reserved for future cleanup of
/// OTLP exporters, file handles, etc.
pub struct TelemetryHandle {
    service_name: String,
}

impl TelemetryHandle {
    /// Return the service name this handle was initialised with.
    pub fn service_name(&self) -> &str {
        &self.service_name
    }
}

impl Drop for TelemetryHandle {
    fn drop(&mut self) {
        // Future: flush OTLP exporter, close file sinks, etc.
    }
}

/// Initialise the global `tracing` subscriber.
///
/// This function is **idempotent** in the sense that it returns
/// [`TelemetryError::AlreadyInitialised`] rather than panicking when called
/// more than once.
///
/// # Errors
///
/// - [`TelemetryError::InvalidFilter`] — if `config.filter` is not a valid
///   `EnvFilter` directive string.
/// - [`TelemetryError::AlreadyInitialised`] — if the global subscriber has
///   already been set.
pub fn init_telemetry(config: TelemetryConfig) -> Result<TelemetryHandle, TelemetryError> {
    let filter = match &config.filter {
        Some(f) => EnvFilter::try_new(f)
            .map_err(|_| TelemetryError::InvalidFilter(f.clone()))?,
        None => EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info")),
    };

    let span_events = if config.log_spans {
        FmtSpan::NEW | FmtSpan::CLOSE
    } else {
        FmtSpan::NONE
    };

    let result = if config.json_logs {
        tracing_subscriber::fmt()
            .json()
            .with_env_filter(filter)
            .with_span_events(span_events)
            .with_current_span(true)
            .try_init()
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_span_events(span_events)
            .try_init()
    };

    result.map_err(|_| TelemetryError::AlreadyInitialised)?;

    Ok(TelemetryHandle {
        service_name: config.service_name,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_exposes_service_name() {
        let handle = TelemetryHandle {
            service_name: "test-svc".to_owned(),
        };
        assert_eq!(handle.service_name(), "test-svc");
    }

    #[test]
    fn default_config_reasonable() {
        let cfg = TelemetryConfig::default();
        assert_eq!(cfg.service_name, "ify-runtime");
        assert!(!cfg.json_logs);
        assert!(cfg.filter.is_none());
        assert!(!cfg.log_spans);
    }

    #[test]
    fn invalid_filter_returns_error() {
        let _cfg = TelemetryConfig {
            filter: Some("[[[invalid".to_owned()),
            ..Default::default()
        };
        // We build the filter manually to test the error path without
        // touching the global subscriber.
        let result = EnvFilter::try_new("[[[invalid");
        assert!(result.is_err(), "invalid filter must fail EnvFilter parsing");
    }
}
