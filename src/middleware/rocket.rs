//! Rocket fairing adapter.
//!
//! # Example
//!
//! ```rust,no_run
//! use apidash::{ApiDashClient, Options};
//! use apidash::middleware::rocket::ApiDashFairing;
//!
//! let client = ApiDashClient::new(Options::new("key", "https://example.com/ingest")).unwrap();
//! let rocket = rocket::build().attach(ApiDashFairing::new(client));
//! ```

use crate::consumer::default_identify_consumer;
use crate::{ApiDashClient, RequestEvent};

use rocket::fairing::{Fairing, Info, Kind};
use rocket::{Data, Request, Response};
use std::sync::Arc;
use std::time::Instant;

/// Rocket fairing that captures request analytics.
pub struct ApiDashFairing {
    client: Arc<ApiDashClient>,
}

impl ApiDashFairing {
    pub fn new(client: Arc<ApiDashClient>) -> Self {
        Self { client }
    }
}

#[rocket::async_trait]
impl Fairing for ApiDashFairing {
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
        let path = req.uri().path().to_string();
        let status = resp.status().code;

        let request_size = req
            .headers()
            .get_one("content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);

        let response_size = resp.body().preset_size().unwrap_or(0);

        let consumer_id =
            default_identify_consumer(|name| req.headers().get_one(name).map(|v| v.to_string()));

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
