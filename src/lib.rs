//! API Usage Dashboard — Rust SDK
//!
//! Buffers request events in memory and flushes them in batches to an
//! ingestion endpoint on a background thread. Includes exponential backoff,
//! disk persistence for undelivered events, and SSRF protection.

mod client;
mod consumer;
pub mod middleware;
mod ssrf;
mod types;

pub use client::PeekApiClient;
pub use consumer::{default_identify_consumer, hash_consumer_id};
pub use ssrf::{is_private_ip, validate_endpoint};
pub use types::{ErrorCallback, IdentifyConsumerFn, Options, RequestEvent};
