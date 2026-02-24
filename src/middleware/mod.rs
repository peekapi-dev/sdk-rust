//! Framework-specific middleware adapters.
//!
//! Each adapter is feature-gated and only compiled when the corresponding
//! feature flag is enabled in `Cargo.toml`.

#[cfg(feature = "actix")]
pub mod actix;

#[cfg(feature = "axum-middleware")]
pub mod axum;

#[cfg(feature = "rocket-fairing")]
pub mod rocket;
