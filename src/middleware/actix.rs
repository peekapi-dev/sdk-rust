//! Actix Web middleware adapter.
//!
//! # Example
//!
//! ```rust,no_run
//! use apidash::{ApiDashClient, Options};
//! use apidash::middleware::actix::ApiDash;
//!
//! let client = ApiDashClient::new(Options::new("key", "https://example.com/ingest")).unwrap();
//! let app = actix_web::App::new().wrap(ApiDash::new(client));
//! ```

use crate::consumer::default_identify_consumer;
use crate::{ApiDashClient, RequestEvent};

use actix_service::{Service, Transform};
use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::Error;
use std::future::{ready, Future, Ready};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

/// Actix Web middleware that captures request analytics.
pub struct ApiDash {
    client: Arc<ApiDashClient>,
}

impl ApiDash {
    pub fn new(client: Arc<ApiDashClient>) -> Self {
        Self { client }
    }
}

impl<S, B> Transform<S, ServiceRequest> for ApiDash
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Transform = ApiDashService<S>;
    type InitError = ();
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ApiDashService {
            service,
            client: Arc::clone(&self.client),
        }))
    }
}

pub struct ApiDashService<S> {
    service: S,
    client: Arc<ApiDashClient>,
}

impl<S, B> Service<ServiceRequest> for ApiDashService<S>
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
        let path = req.path().to_string();
        let request_size = req
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);

        let consumer_id = default_identify_consumer(|name| {
            req.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.to_string())
        });

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
