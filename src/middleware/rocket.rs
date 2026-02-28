//! Rocket fairing adapter.
//!
//! # Example
//!
//! ```rust,no_run
//! use peekapi::{PeekApiClient, Options};
//! use peekapi::middleware::rocket::PeekApiFairing;
//!
//! let client = PeekApiClient::new(Options::new("key", "https://example.com/ingest")).unwrap();
//! let rocket = rocket::build().attach(PeekApiFairing::new(client));
//! ```

use crate::consumer::default_identify_consumer;
use crate::{PeekApiClient, RequestEvent};

use rocket::fairing::{Fairing, Info, Kind};
use rocket::{Data, Request, Response};
use std::sync::Arc;
use std::time::Instant;

/// Rocket fairing that captures request analytics.
pub struct PeekApiFairing {
    client: Arc<PeekApiClient>,
}

impl PeekApiFairing {
    pub fn new(client: Arc<PeekApiClient>) -> Self {
        Self { client }
    }
}

#[rocket::async_trait]
impl Fairing for PeekApiFairing {
    fn info(&self) -> Info {
        Info {
            name: "API Dashboard Analytics",
            kind: Kind::Request | Kind::Response,
        }
    }

    async fn on_request(&self, req: &mut Request<'_>, _data: &mut Data<'_>) {
        // Store the start time in local cache
        req.local_cache(Instant::now);
    }

    async fn on_response<'r>(&self, req: &'r Request<'_>, resp: &mut Response<'r>) {
        let start = *req.local_cache(Instant::now);
        let elapsed = start.elapsed();

        let method = req.method().as_str().to_string();
        let mut path = req.uri().path().to_string();
        if self.client.collect_query_string() {
            if let Some(qs) = req.uri().query() {
                let qs_str = qs.as_str();
                if !qs_str.is_empty() {
                    let mut params: Vec<&str> = qs_str.split('&').collect();
                    params.sort();
                    path.push('?');
                    path.push_str(&params.join("&"));
                }
            }
        }
        let status = resp.status().code;

        let request_size = req
            .headers()
            .get_one("content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);

        let response_size = resp.body().preset_size().unwrap_or(0);

        let get_header = |name: &str| req.headers().get_one(name).map(|v| v.to_string());
        let consumer_id = if let Some(ref cb) = self.client.identify_consumer() {
            cb(&get_header)
        } else {
            default_identify_consumer(get_header)
        };

        self.client.track(RequestEvent {
            method,
            path,
            status_code: status,
            response_time_ms: elapsed.as_secs_f64() * 1000.0,
            request_size,
            response_size,
            consumer_id,
            metadata: None,
            timestamp: String::new(),
        });
    }
}
