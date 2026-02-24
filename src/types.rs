use serde::{Deserialize, Serialize};
use std::time::Duration;

/// A single captured API request event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestEvent {
    pub method: String,
    pub path: String,
    pub status_code: u16,
    pub response_time_ms: f64,
    #[serde(default)]
    pub request_size: usize,
    #[serde(default)]
    pub response_size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub timestamp: String,
}

/// Configuration for the API dashboard client.
pub struct Options {
    /// API key for authenticating with the ingestion endpoint (required).
    pub api_key: String,
    /// URL of the ingestion endpoint (required).
    pub endpoint: String,
    /// Time between automatic flushes. Default: 10s.
    pub flush_interval: Duration,
    /// Number of events that triggers an automatic flush. Default: 100.
    pub batch_size: usize,
    /// Maximum number of events held in memory. Default: 10,000.
    pub max_buffer_size: usize,
    /// Maximum size of the storage file in bytes. Default: 5MB.
    pub max_storage_bytes: u64,
    /// Maximum size of a single serialized event in bytes. Default: 64KB.
    pub max_event_bytes: usize,
    /// Enable debug logging to stderr.
    pub debug: bool,
    /// File path for persisting undelivered events.
    /// Default: `<temp_dir>/apidash-events-<hash>.jsonl`
    pub storage_path: Option<String>,
    /// Optional error callback invoked from the background thread.
    pub on_error: Option<Box<dyn Fn(&dyn std::error::Error) + Send + Sync>>,
}

impl Options {
    /// Create options with required fields only; all others use defaults.
    pub fn new(api_key: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            endpoint: endpoint.into(),
            flush_interval: Duration::from_secs(10),
            batch_size: 100,
            max_buffer_size: 10_000,
            max_storage_bytes: 5_242_880,
            max_event_bytes: 65_536,
            debug: false,
            storage_path: None,
            on_error: None,
        }
    }
}
