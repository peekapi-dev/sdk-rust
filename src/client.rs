use crate::ssrf::validate_endpoint;
use crate::types::{ErrorCallback, Options, RequestEvent};

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const MAX_PATH_LENGTH: usize = 2048;
const MAX_METHOD_LENGTH: usize = 16;
const MAX_CONSUMER_ID_LENGTH: usize = 256;
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
const BASE_BACKOFF: Duration = Duration::from_secs(1);
const SEND_TIMEOUT: Duration = Duration::from_secs(5);

struct Inner {
    buffer: Vec<RequestEvent>,
    spare: Vec<RequestEvent>,
    consecutive_failures: u32,
    backoff_until: Instant,
    flush_in_flight: bool,
    wake: bool, // condvar predicate — set when flush or shutdown is requested
}

/// Buffered analytics client.
///
/// Events are accumulated in memory and flushed to the ingestion endpoint
/// on a background thread. Undelivered events are persisted to disk (JSONL)
/// and recovered on the next startup.
pub struct ApiDashClient {
    inner: Mutex<Inner>,
    cond: Condvar,
    closed: AtomicBool,
    opts: ClientOpts,
    // Background thread handle — joined on shutdown
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// Immutable configuration extracted from Options (no callbacks).
struct ClientOpts {
    api_key: String,
    endpoint: String,
    flush_interval: Duration,
    batch_size: usize,
    max_buffer_size: usize,
    max_storage_bytes: u64,
    max_event_bytes: usize,
    debug: bool,
    storage_path: String,
    on_error: Option<ErrorCallback>,
}

impl ApiDashClient {
    /// Create a new client with the given options.
    ///
    /// Validates the configuration, loads any previously persisted events
    /// from disk, and starts a background thread for periodic flushing.
    pub fn new(opts: Options) -> Result<Arc<Self>, String> {
        if opts.api_key.is_empty() {
            return Err("[apidash] 'api_key' is required".to_string());
        }
        if opts.api_key.contains('\0') || opts.api_key.contains('\r') || opts.api_key.contains('\n')
        {
            return Err("[apidash] 'api_key' contains invalid characters".to_string());
        }

        let endpoint = validate_endpoint(&opts.endpoint)?;

        let storage_path = opts.storage_path.unwrap_or_else(|| {
            use sha2::{Digest, Sha256};
            let hash = Sha256::digest(endpoint.as_bytes());
            let hex: String = hash[..4].iter().map(|b| format!("{b:02x}")).collect();
            let dir = std::env::temp_dir();
            dir.join(format!("apidash-events-{hex}.jsonl"))
                .to_string_lossy()
                .to_string()
        });

        let batch_size = if opts.batch_size == 0 {
            100
        } else {
            opts.batch_size
        };

        let client_opts = ClientOpts {
            api_key: opts.api_key,
            endpoint,
            flush_interval: opts.flush_interval,
            batch_size,
            max_buffer_size: if opts.max_buffer_size == 0 {
                10_000
            } else {
                opts.max_buffer_size
            },
            max_storage_bytes: if opts.max_storage_bytes == 0 {
                5_242_880
            } else {
                opts.max_storage_bytes
            },
            max_event_bytes: if opts.max_event_bytes == 0 {
                65_536
            } else {
                opts.max_event_bytes
            },
            debug: opts.debug,
            storage_path,
            on_error: opts.on_error,
        };

        let inner = Inner {
            buffer: Vec::with_capacity(batch_size),
            spare: Vec::with_capacity(batch_size),
            consecutive_failures: 0,
            backoff_until: Instant::now(),
            flush_in_flight: false,
            wake: false,
        };

        let client = Arc::new(Self {
            inner: Mutex::new(inner),
            cond: Condvar::new(),
            closed: AtomicBool::new(false),
            opts: client_opts,
            thread: Mutex::new(None),
        });

        client.load_from_disk();

        // Spawn background flush thread
        let c = Arc::clone(&client);
        let handle = std::thread::Builder::new()
            .name("apidash-flush".to_string())
            .spawn(move || c.background_loop())
            .map_err(|e| format!("[apidash] Failed to spawn flush thread: {e}"))?;

        *client.thread.lock().unwrap() = Some(handle);
        Ok(client)
    }

    /// Buffer an analytics event. Never panics on error — drops silently
    /// (or logs if debug is enabled).
    pub fn track(&self, mut event: RequestEvent) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }

        // Sanitize input lengths
        if event.method.len() > MAX_METHOD_LENGTH {
            event.method.truncate(MAX_METHOD_LENGTH);
        }
        event.method = event.method.to_uppercase();

        if event.path.len() > MAX_PATH_LENGTH {
            event.path.truncate(MAX_PATH_LENGTH);
        }

        if let Some(ref mut cid) = event.consumer_id {
            if cid.len() > MAX_CONSUMER_ID_LENGTH {
                cid.truncate(MAX_CONSUMER_ID_LENGTH);
            }
        }

        // Timestamp
        if event.timestamp.is_empty() {
            event.timestamp = now_iso8601();
        }

        // Per-event size limit
        if let Ok(raw) = serde_json::to_vec(&event) {
            if raw.len() > self.opts.max_event_bytes {
                // Strip metadata and retry
                event.metadata = None;
                if let Ok(raw2) = serde_json::to_vec(&event) {
                    if raw2.len() > self.opts.max_event_bytes {
                        if self.opts.debug {
                            eprintln!("[apidash] Event too large, dropping ({} bytes)", raw2.len());
                        }
                        return;
                    }
                }
            }
        }

        let mut guard = self.inner.lock().unwrap();
        if guard.buffer.len() >= self.opts.max_buffer_size {
            // Buffer full — signal flush
            guard.wake = true;
            self.cond.notify_one();
            return;
        }
        guard.buffer.push(event);
        let should_flush = guard.buffer.len() >= self.opts.batch_size;
        if should_flush {
            guard.wake = true;
        }
        drop(guard);

        if should_flush {
            self.cond.notify_one();
        }
    }

    /// Flush buffered events synchronously. Respects in-flight and backoff guards.
    pub fn flush(&self) {
        let events = {
            let mut guard = self.inner.lock().unwrap();
            if guard.flush_in_flight {
                return;
            }
            if guard.consecutive_failures > 0 && Instant::now() < guard.backoff_until {
                return;
            }
            if guard.buffer.is_empty() {
                return;
            }
            guard.flush_in_flight = true;

            // Double-buffer swap: take spare first to avoid double borrow
            let spare = std::mem::take(&mut guard.spare);

            std::mem::replace(&mut guard.buffer, spare)
        };

        let result = self.send(&events);

        let mut guard = self.inner.lock().unwrap();
        guard.flush_in_flight = false;

        match result {
            Ok(()) => {
                guard.consecutive_failures = 0;
                guard.backoff_until = Instant::now();
                // Recycle the events vec as spare
                let mut recycled = events;
                recycled.clear();
                if guard.spare.is_empty() {
                    guard.spare = recycled;
                }
                if self.opts.debug {
                    eprintln!("[apidash] Flushed events successfully");
                }
            }
            Err(ref e) if !is_retryable(e) => {
                drop(guard);
                self.persist_to_disk(&events);
                if self.opts.debug {
                    eprintln!("[apidash] Non-retryable error, persisted to disk: {e}");
                }
                self.call_on_error(e);
            }
            Err(ref e) => {
                guard.consecutive_failures += 1;
                let failures = guard.consecutive_failures;

                if failures >= MAX_CONSECUTIVE_FAILURES {
                    guard.consecutive_failures = 0;
                    drop(guard);
                    self.persist_to_disk(&events);
                } else {
                    // Re-insert events at the front
                    let space = self.opts.max_buffer_size.saturating_sub(guard.buffer.len());
                    let reinsert_count = events.len().min(space);
                    if reinsert_count > 0 {
                        let mut merged = Vec::with_capacity(reinsert_count + guard.buffer.len());
                        merged.extend_from_slice(&events[..reinsert_count]);
                        merged.append(&mut guard.buffer);
                        guard.buffer = merged;
                    }

                    // Exponential backoff with jitter
                    let base = BASE_BACKOFF * (1 << (failures - 1));
                    let jitter = 0.5 + rand_f64() * 0.5;
                    let delay = Duration::from_secs_f64(base.as_secs_f64() * jitter);
                    guard.backoff_until = Instant::now() + delay;
                    drop(guard);
                }

                if self.opts.debug {
                    eprintln!("[apidash] Flush failed: {e}");
                }
                self.call_on_error(e);
            }
        }
    }

    /// Graceful shutdown: stop background thread, final flush, persist remainder.
    pub fn shutdown(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return; // Already closed
        }

        // Wake the background thread so it exits
        {
            let mut guard = self.inner.lock().unwrap();
            guard.wake = true;
        }
        self.cond.notify_one();

        // Join background thread
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }

        // Reset flush_in_flight so flush() can proceed
        {
            let mut guard = self.inner.lock().unwrap();
            guard.flush_in_flight = false;
        }

        // Final flush
        self.flush();

        // Persist any remaining events
        let remaining = {
            let mut guard = self.inner.lock().unwrap();
            std::mem::take(&mut guard.buffer)
        };
        if !remaining.is_empty() {
            self.persist_to_disk(&remaining);
        }
    }

    /// Current number of events in the buffer (for testing).
    pub fn buffer_len(&self) -> usize {
        self.inner.lock().unwrap().buffer.len()
    }

    // ------------------------------------------------------------------
    // Background thread
    // ------------------------------------------------------------------

    fn background_loop(&self) {
        loop {
            // Wait until woken or flush interval elapses
            let guard = self.inner.lock().unwrap();
            let (mut guard, _) = self
                .cond
                .wait_timeout_while(guard, self.opts.flush_interval, |inner| !inner.wake)
                .unwrap();
            guard.wake = false;
            drop(guard);

            if self.closed.load(Ordering::Relaxed) {
                break;
            }

            self.flush();
        }
    }

    // ------------------------------------------------------------------
    // Network
    // ------------------------------------------------------------------

    fn send(&self, events: &[RequestEvent]) -> Result<(), SendError> {
        let body = serde_json::to_vec(events)
            .map_err(|e| SendError::new(format!("JSON marshal failed: {e}"), false))?;

        let result = ureq::post(&self.opts.endpoint)
            .timeout(SEND_TIMEOUT)
            .set("Content-Type", "application/json")
            .set("x-api-key", &self.opts.api_key)
            .set(
                "x-apidash-sdk",
                &format!("rust/{}", env!("CARGO_PKG_VERSION")),
            )
            .send_bytes(&body);

        match result {
            Ok(resp) => {
                let status = resp.status();
                if !(200..300).contains(&status) {
                    let retryable = status == 429 || status >= 500;
                    return Err(SendError::new(
                        format!("Ingestion API returned {status}"),
                        retryable,
                    ));
                }
                Ok(())
            }
            Err(ureq::Error::Status(status, _resp)) => {
                let retryable = status == 429 || status >= 500;
                Err(SendError::new(
                    format!("Ingestion API returned {status}"),
                    retryable,
                ))
            }
            Err(ureq::Error::Transport(e)) => {
                Err(SendError::new(format!("Transport error: {e}"), true))
            }
        }
    }

    // ------------------------------------------------------------------
    // Disk persistence
    // ------------------------------------------------------------------

    fn persist_to_disk(&self, events: &[RequestEvent]) {
        if events.is_empty() {
            return;
        }

        // Check file size
        let current_size = fs::metadata(&self.opts.storage_path)
            .map(|m| m.len())
            .unwrap_or(0);
        if current_size >= self.opts.max_storage_bytes {
            if self.opts.debug {
                eprintln!(
                    "[apidash] Storage file full ({current_size} bytes), skipping disk persist of {} events",
                    events.len()
                );
            }
            return;
        }

        let data = match serde_json::to_string(events) {
            Ok(d) => d,
            Err(e) => {
                if self.opts.debug {
                    eprintln!("[apidash] Failed to marshal events for disk: {e}");
                }
                return;
            }
        };

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.opts.storage_path);

        match file {
            Ok(mut f) => {
                if let Err(e) = writeln!(f, "{data}") {
                    if self.opts.debug {
                        eprintln!("[apidash] Failed to write events to disk: {e}");
                    }
                } else if self.opts.debug {
                    eprintln!(
                        "[apidash] Persisted {} events to {}",
                        events.len(),
                        self.opts.storage_path
                    );
                }
            }
            Err(e) => {
                if self.opts.debug {
                    eprintln!("[apidash] Failed to open storage file: {e}");
                }
            }
        }
    }

    fn load_from_disk(&self) {
        let file = match fs::File::open(&self.opts.storage_path) {
            Ok(f) => f,
            Err(_) => return, // file doesn't exist
        };

        let reader = BufReader::new(file);
        let mut loaded = 0usize;
        let mut guard = self.inner.lock().unwrap();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let batch: Vec<RequestEvent> = match serde_json::from_str(&line) {
                Ok(b) => b,
                Err(_) => continue, // skip corrupt lines
            };

            for event in batch {
                if guard.buffer.len() >= self.opts.max_buffer_size {
                    break;
                }
                guard.buffer.push(event);
                loaded += 1;
            }
            if guard.buffer.len() >= self.opts.max_buffer_size {
                break;
            }
        }

        drop(guard);

        // Remove the file after loading
        let _ = fs::remove_file(&self.opts.storage_path);

        if self.opts.debug && loaded > 0 {
            eprintln!("[apidash] Recovered {loaded} events from disk");
        }
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn call_on_error(&self, err: &dyn std::error::Error) {
        if let Some(ref cb) = self.opts.on_error {
            cb(err);
        }
    }
}

impl Drop for ApiDashClient {
    fn drop(&mut self) {
        if !self.closed.load(Ordering::Relaxed) {
            self.shutdown();
        }
    }
}

// ------------------------------------------------------------------
// Error types
// ------------------------------------------------------------------

#[derive(Debug)]
struct SendError {
    message: String,
    retryable: bool,
}

impl SendError {
    fn new(message: String, retryable: bool) -> Self {
        Self { message, retryable }
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SendError {}

fn is_retryable(err: &SendError) -> bool {
    err.retryable
}

// ------------------------------------------------------------------
// Utilities
// ------------------------------------------------------------------

fn now_iso8601() -> String {
    // Simple UTC timestamp without pulling in chrono
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Convert to date-time components
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    let millis = duration.subsec_millis();

    // Calculate year/month/day from days since epoch (1970-01-01)
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Civil date from days since 1970-01-01 (algorithm from Howard Hinnant)
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Simple pseudo-random f64 in [0, 1) for backoff jitter.
/// Not cryptographic — just needs to spread retries.
fn rand_f64() -> f64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // xorshift32
    let mut x = seed.wrapping_add(1); // avoid zero
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    (x as f64) / (u32::MAX as f64)
}
