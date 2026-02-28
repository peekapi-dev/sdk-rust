//! Actix Web middleware adapter.
//!
//! # Example
//!
//! ```rust,no_run
//! use peekapi::{PeekApiClient, Options};
//! use peekapi::middleware::actix::PeekApi;
//!
//! let client = PeekApiClient::new(Options::new("key", "https://example.com/ingest")).unwrap();
//! let app = actix_web::App::new().wrap(PeekApi::new(client));
//! ```

use crate::consumer::default_identify_consumer;
use crate::{PeekApiClient, RequestEvent};

use actix_service::{Service, Transform};
use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::Error;
use std::future::{ready, Future, Ready};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

/// Actix Web middleware that captures request analytics.
pub struct PeekApi {
    client: Arc<PeekApiClient>,
}

impl PeekApi {
    pub fn new(client: Arc<PeekApiClient>) -> Self {
        Self { client }
    }
}

impl<S, B> Transform<S, ServiceRequest> for PeekApi
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = PeekApiMiddleware<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(PeekApiMiddleware {
            service,
            client: Arc::clone(&self.client),
        }))
    }
}

pub struct PeekApiMiddleware<S> {
    service: S,
    client: Arc<PeekApiClient>,
}

impl<S, B> Service<ServiceRequest> for PeekApiMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(
        &self,
        ctx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let start = Instant::now();
        let method = req.method().to_string();
        let mut path = req.path().to_string();
        if self.client.collect_query_string() {
            let qs = req.query_string();
            if !qs.is_empty() {
                let mut params: Vec<&str> = qs.split('&').collect();
                params.sort();
                path.push('?');
                path.push_str(&params.join("&"));
            }
        }
        let request_size = req
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);

        let get_header = |name: &str| {
            req.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string())
        };
        let consumer_id = if let Some(ref cb) = self.client.identify_consumer() {
            cb(&get_header)
        } else {
            default_identify_consumer(get_header)
        };

        let client = Arc::clone(&self.client);
        let fut = self.service.call(req);

        Box::pin(async move {
            let result = fut.await;

            match result {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let response_size = resp
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);

                    let elapsed = start.elapsed();
                    client.track(RequestEvent {
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

                    Ok(resp)
                }
                Err(e) => Err(e),
            }
        })
    }
}
