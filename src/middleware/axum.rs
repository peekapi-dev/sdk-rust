//! Axum middleware adapter (Tower Layer/Service).
//!
//! # Example
//!
//! ```rust,no_run
//! use apidash::{ApiDashClient, Options};
//! use apidash::middleware::axum::ApiDashLayer;
//! use axum::Router;
//!
//! let client = ApiDashClient::new(Options::new("key", "https://example.com/ingest")).unwrap();
//! let app = Router::new().layer(ApiDashLayer::new(client));
//! ```

use crate::consumer::default_identify_consumer;
use crate::{ApiDashClient, RequestEvent};

use axum::body::Body;
use http::Request;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::{Layer, Service};

/// Tower Layer that wraps services with API analytics tracking.
#[derive(Clone)]
pub struct ApiDashLayer {
    client: Arc<ApiDashClient>,
}

impl ApiDashLayer {
    pub fn new(client: Arc<ApiDashClient>) -> Self {
        Self { client }
    }
}

impl<S> Layer<S> for ApiDashLayer {
    type Service = ApiDashService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ApiDashService {
            inner,
            client: Arc::clone(&self.client),
        }
    }
}

/// Tower Service that captures request analytics.
#[derive(Clone)]
pub struct ApiDashService<S> {
    inner: S,
    client: Arc<ApiDashClient>,
}

impl<S> Service<Request<Body>> for ApiDashService<S>
where
    S: Service<Request<Body>, Response = axum::response::Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Response = axum::response::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let start = Instant::now();
        let method = req.method().to_string();
        let path = req.uri().path().to_string();
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

        let future = self.inner.call(req);

        ResponseFuture {
            inner: future,
            client: Arc::clone(&self.client),
            start,
            method,
            path,
            request_size,
            consumer_id,
        }
    }
}

pin_project! {
    /// Future that tracks response metadata after the inner service completes.
    pub struct ResponseFuture<F> {
        #[pin]
        inner: F,
        client: Arc<ApiDashClient>,
        start: Instant,
        method: String,
        path: String,
        request_size: usize,
        consumer_id: Option<String>,
    }
}

impl<F, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<axum::response::Response, E>>,
{
    type Output = Result<axum::response::Response, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match this.inner.poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                if let Ok(ref resp) = result {
                    let status = resp.status().as_u16();
                    let response_size = resp
                        .headers()
                        .get("content-length")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);

                    let elapsed = this.start.elapsed();
                    this.client.track(RequestEvent {
                        method: std::mem::take(this.method),
                        path: std::mem::take(this.path),
                        status_code: status,
                        response_time_ms: elapsed.as_secs_f64() * 1000.0,
                        request_size: *this.request_size,
                        response_size,
                        consumer_id: this.consumer_id.take(),
                        metadata: None,
                        timestamp: String::new(),
                    });
                }
                Poll::Ready(result)
            }
        }
    }
}
