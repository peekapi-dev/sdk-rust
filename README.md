# PeekAPI — Rust SDK

[![crates.io](https://img.shields.io/crates/v/peekapi)](https://crates.io/crates/peekapi)
[![license](https://img.shields.io/crates/l/peekapi)](./LICENSE)

Rust SDK for [PeekAPI](https://peekapi.dev). Thread-based buffered client with feature-gated middleware for Actix Web, Axum, and Rocket.

## Install

```toml
# Cargo.toml
[dependencies]
peekapi = "0.1"

# Enable framework middleware (pick one or more)
# peekapi = { version = "0.1", features = ["actix"] }
# peekapi = { version = "0.1", features = ["axum-middleware"] }
# peekapi = { version = "0.1", features = ["rocket-fairing"] }
```

## Quick Start

### Actix Web

```rust
use peekapi::{PeekApiClient, Options};
use peekapi::middleware::actix::PeekApi;
use actix_web::{web, App, HttpServer, HttpResponse};
use std::sync::Arc;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let client = PeekApiClient::new(Options::new("ak_live_xxx", "")).unwrap();

    HttpServer::new(move || {
        App::new()
            .wrap(PeekApi::new(Arc::clone(&client)))
            .route("/api/hello", web::get().to(|| async { HttpResponse::Ok().json(serde_json::json!({"message": "hello"})) }))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
```

### Axum

```rust
use peekapi::{PeekApiClient, Options};
use peekapi::middleware::axum::PeekApiLayer;
use axum::{Router, routing::get, Json};

#[tokio::main]
async fn main() {
    let client = PeekApiClient::new(Options::new("ak_live_xxx", "")).unwrap();

    let app = Router::new()
        .route("/api/hello", get(|| async { Json(serde_json::json!({"message": "hello"})) }))
        .layer(PeekApiLayer::new(client));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
```

### Rocket

```rust
use peekapi::{PeekApiClient, Options};
use peekapi::middleware::rocket::PeekApiFairing;
use rocket::{get, routes};

#[get("/api/hello")]
fn hello() -> &'static str {
    r#"{"message":"hello"}"#
}

#[rocket::main]
async fn main() {
    let client = PeekApiClient::new(Options::new("ak_live_xxx", "")).unwrap();

    rocket::build()
        .attach(PeekApiFairing::new(client))
        .mount("/", routes![hello])
        .launch()
        .await
        .unwrap();
}
```

### Standalone Client

```rust
use peekapi::{PeekApiClient, Options, RequestEvent};

let client = PeekApiClient::new(Options::new("ak_live_xxx", "")).unwrap();

client.track(RequestEvent {
    method: "GET".into(),
    path: "/api/users".into(),
    status_code: 200,
    response_time_ms: 42.0,
    ..Default::default()
});

// Graceful shutdown (flushes remaining events, persists to disk)
client.shutdown();
```

## Configuration

| Field | Type | Default | Description |
|---|---|---|---|
| `api_key` | `String` | required | Your PeekAPI key |
| `endpoint` | `String` | PeekAPI cloud | Ingestion endpoint URL |
| `flush_interval` | `Duration` | `10s` | Time between automatic flushes |
| `batch_size` | `usize` | `100` | Events per batch (triggers flush) |
| `max_buffer_size` | `usize` | `10,000` | Max events held in memory |
| `max_storage_bytes` | `u64` | `5MB` | Max disk fallback file size |
| `max_event_bytes` | `usize` | `64KB` | Per-event size limit |
| `storage_path` | `Option<String>` | temp dir | JSONL fallback file path |
| `debug` | `bool` | `false` | Enable debug logging to stderr |
| `on_error` | `Option<ErrorCallback>` | `None` | Callback for background flush errors |

## How It Works

1. Middleware intercepts every request/response
2. Captures method, path, status code, response time, request/response sizes, consumer ID
3. Events are buffered in memory and flushed in batches on a background thread
4. On network failure: exponential backoff with jitter, up to 5 retries
5. After max retries: events are persisted to a JSONL file on disk
6. On next startup: persisted events are recovered and re-sent
7. On shutdown: remaining buffer is flushed or persisted to disk

## Consumer Identification

By default, consumers are identified by:

1. `X-API-Key` header — stored as-is
2. `Authorization` header — hashed with SHA-256 (stored as `hash_<hex>`)

Override with the `identify_consumer` option to use any header:

```rust
let mut opts = Options::new("your-api-key", "https://...");
opts.identify_consumer = Some(Box::new(|get_header| get_header("x-tenant-id")));
let client = PeekApiClient::new(opts).unwrap();
```

The callback receives a header-getter closure (`&dyn Fn(&str) -> Option<String>`) and should return an `Option<String>`.

## Features

- **Minimal dependencies** — serde, serde_json, ureq, sha2 (framework deps are feature-gated)
- **Background thread** — dedicated flush thread with configurable interval and batch size
- **Disk persistence** — undelivered events saved to JSONL, recovered on restart
- **Exponential backoff** — with jitter on network failures
- **SSRF protection** — private IP blocking, HTTPS enforcement (HTTP only for localhost)
- **Input sanitization** — path (2048), method (16), consumer_id (256) truncation
- **Per-event size limit** — strips metadata first, drops if still too large (default 64KB)
- **Feature-gated middleware** — only compile the framework adapter you need

## Feature Flags

| Feature | Framework | Adds |
|---|---|---|
| `actix` | Actix Web 4 | `actix-web`, `actix-service` |
| `axum-middleware` | Axum 0.8 | `axum`, `tower`, `tower-layer`, `http`, `pin-project-lite` |
| `rocket-fairing` | Rocket 0.5 | `rocket` |

## Requirements

- Rust 2021 edition

## License

MIT
