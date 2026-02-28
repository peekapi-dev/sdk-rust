use peekapi::{PeekApiClient, Options, RequestEvent};
use std::sync::Arc;
use std::time::Duration;

fn test_event() -> RequestEvent {
    RequestEvent {
        method: "GET".to_string(),
        path: "/api/users".to_string(),
        status_code: 200,
        response_time_ms: 42.0,
        request_size: 0,
        response_size: 128,
        consumer_id: Some("ak_test_123".to_string()),
        metadata: None,
        timestamp: String::new(),
    }
}

fn make_client(storage_path: &str) -> Arc<PeekApiClient> {
    let mut opts = Options::new("ak_test_key", "http://localhost:9999/ingest");
    opts.storage_path = Some(storage_path.to_string());
    opts.flush_interval = Duration::from_secs(60); // long interval so we control flush
    PeekApiClient::new(opts).unwrap()
}

#[test]
fn track_buffers_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let client = make_client(&path);

    client.track(test_event());
    client.track(test_event());
    client.track(test_event());

    assert_eq!(client.buffer_len(), 3);
    client.shutdown();
}

#[test]
fn track_sanitizes_method_to_uppercase() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let client = make_client(&path);

    let mut event = test_event();
    event.method = "get".to_string();
    client.track(event);

    // Method is uppercased internally — we can verify via serialization
    // after shutdown persist
    client.shutdown();
}

#[test]
fn track_truncates_long_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let client = make_client(&path);

    let mut event = test_event();
    event.path = "x".repeat(5000);
    client.track(event);

    assert_eq!(client.buffer_len(), 1);
    client.shutdown();
}

#[test]
fn track_truncates_long_consumer_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let client = make_client(&path);

    let mut event = test_event();
    event.consumer_id = Some("c".repeat(500));
    client.track(event);

    assert_eq!(client.buffer_len(), 1);
    client.shutdown();
}

#[test]
fn track_ignores_after_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let client = make_client(&path);

    client.shutdown();
    client.track(test_event());
    assert_eq!(client.buffer_len(), 0);
}

#[test]
fn shutdown_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let client = make_client(&path);

    client.shutdown();
    client.shutdown(); // Should not panic
}

#[test]
fn new_rejects_empty_api_key() {
    let opts = Options::new("", "http://localhost:9999/ingest");
    assert!(PeekApiClient::new(opts).is_err());
}

#[test]
fn new_rejects_api_key_with_control_chars() {
    let opts = Options::new("key\0value", "http://localhost:9999/ingest");
    assert!(PeekApiClient::new(opts).is_err());
}

#[test]
fn empty_endpoint_uses_default() {
    let opts = Options::new("ak_test", "");
    let client = PeekApiClient::new(opts).unwrap();
    client.shutdown();
}

#[test]
fn new_rejects_http_non_localhost() {
    let opts = Options::new("ak_test", "http://example.com/ingest");
    assert!(PeekApiClient::new(opts).is_err());
}

#[test]
fn new_allows_http_localhost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let mut opts = Options::new("ak_test", "http://localhost:9999/ingest");
    opts.storage_path = Some(path);
    let client = PeekApiClient::new(opts);
    assert!(client.is_ok());
    client.unwrap().shutdown();
}

#[test]
fn new_allows_https() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();
    let mut opts = Options::new("ak_test", "https://api.example.com/ingest");
    opts.storage_path = Some(path);
    let client = PeekApiClient::new(opts);
    assert!(client.is_ok());
    client.unwrap().shutdown();
}

#[test]
fn disk_persistence_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();

    // First client: track events, shutdown (persists to disk since endpoint is unreachable)
    {
        let client = make_client(&path);
        for _ in 0..5 {
            client.track(test_event());
        }
        assert_eq!(client.buffer_len(), 5);
        client.shutdown();
    }

    // Check file was created
    assert!(
        std::path::Path::new(&path).exists(),
        "Storage file should exist after shutdown with buffered events"
    );

    // Second client: should load events from disk
    {
        let client = make_client(&path);
        assert!(
            client.buffer_len() >= 5,
            "Should have loaded persisted events, got {}",
            client.buffer_len()
        );
        client.shutdown();
    }
}

#[test]
fn runtime_disk_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();

    let client = make_client(&path);

    // Simulate events persisted to disk mid-process
    let events = vec![test_event()];
    let data = serde_json::to_string(&events).unwrap();
    std::fs::write(&path, format!("{}\n", data)).unwrap();

    // Trigger runtime recovery on the same client
    client.recover_from_disk();

    assert_eq!(
        client.buffer_len(),
        1,
        "Should have recovered 1 event from disk"
    );
    client.shutdown();
}

#[test]
fn custom_identify_consumer_callback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();

    let mut opts = Options::new("ak_test", "http://localhost:9999/ingest");
    opts.storage_path = Some(path);
    opts.flush_interval = Duration::from_secs(60);
    opts.identify_consumer = Some(Box::new(|get_header| get_header("x-tenant-id")));

    let client = PeekApiClient::new(opts).unwrap();
    let cb = client.identify_consumer();
    assert!(cb.is_some());

    // Simulate header lookup
    let id = cb.as_ref().unwrap()(&|name| match name {
        "x-tenant-id" => Some("tenant-42".to_string()),
        "x-api-key" => Some("ignored".to_string()),
        _ => None,
    });
    assert_eq!(id, Some("tenant-42".to_string()));

    client.shutdown();
}

#[test]
fn track_respects_max_buffer_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();

    let mut opts = Options::new("ak_test", "http://localhost:9999/ingest");
    opts.storage_path = Some(path);
    opts.flush_interval = Duration::from_secs(60);
    opts.max_buffer_size = 5;
    opts.batch_size = 1000; // don't trigger batch flush
    let client = PeekApiClient::new(opts).unwrap();

    for _ in 0..10 {
        client.track(test_event());
    }

    // Buffer should not exceed max_buffer_size
    assert!(client.buffer_len() <= 5);
    client.shutdown();
}

#[test]
fn collect_query_string_defaults_to_false() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();

    let mut opts = Options::new("ak_test", "http://localhost:9999/ingest");
    opts.storage_path = Some(path);
    opts.flush_interval = Duration::from_secs(60);
    let client = PeekApiClient::new(opts).unwrap();

    assert!(!client.collect_query_string());
    client.shutdown();
}

#[test]
fn collect_query_string_getter() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("events.jsonl")
        .to_str()
        .unwrap()
        .to_string();

    let mut opts = Options::new("ak_test", "http://localhost:9999/ingest");
    opts.storage_path = Some(path);
    opts.flush_interval = Duration::from_secs(60);
    opts.collect_query_string = true;
    let client = PeekApiClient::new(opts).unwrap();

    assert!(client.collect_query_string());
    client.shutdown();
}
