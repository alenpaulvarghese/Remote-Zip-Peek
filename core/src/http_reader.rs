use crate::archive_limits::{check_directory, MAX_DIRECTORY_BYTES, TAIL_BYTES};
use crate::error::{Error, Result};
use bytes::Bytes;
use futures::future::BoxFuture;
use futures::task::{Context, Poll};
use reqwest::header::{HeaderMap, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, RANGE};
use reqwest::{Client, Response, StatusCode};
use std::cmp::min;
use std::io::SeekFrom;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

const USER_AGENT: &str = concat!("lazy-zip/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_CHUNK_BYTES: u64 = 1024 * 1024;
const MAX_RETRY_AFTER: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// Total tries for each request, including the first.
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(10),
        }
    }
}

impl RetryPolicy {
    fn delay(&self, attempt: u32) -> Duration {
        let doubled = self.base_delay.saturating_mul(1 << attempt.min(16));
        let delay = min(doubled, self.max_delay);
        // Up to 25% jitter, so parallel downloads don't all retry at the same moment.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.subsec_nanos());
        delay + delay.mul_f64(f64::from(nanos % 1000) / 4000.0)
    }
}

/// How readers connect. Cloning shares the connection pool and the network byte count.
#[derive(Clone)]
pub struct ReaderConfig {
    pub client: Client,
    pub retry: RetryPolicy,
    pub chunk_bytes: u64,
    pub network_bytes: Arc<AtomicU64>,
}

impl ReaderConfig {
    pub fn new() -> Result<Self> {
        Self::with_timeouts(CONNECT_TIMEOUT, READ_TIMEOUT)
    }

    pub fn with_timeouts(connect: Duration, read: Duration) -> Result<Self> {
        // There is no overall timeout, because large downloads legitimately take a long time.
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(connect)
            .read_timeout(read)
            .build()?;
        Ok(Self {
            client,
            retry: RetryPolicy::default(),
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            network_bytes: Arc::default(),
        })
    }
}

/// Identifies one version of the remote file.
#[derive(Clone, Debug, PartialEq)]
enum Validator {
    ETag(String),
    LastModified(String),
}

impl Validator {
    fn from_headers(headers: &HeaderMap) -> Option<Self> {
        let text = |name| headers.get(name)?.to_str().ok().map(str::to_string);
        text(reqwest::header::ETAG)
            .map(Validator::ETag)
            .or_else(|| text(reqwest::header::LAST_MODIFIED).map(Validator::LastModified))
    }
}

struct Remote {
    config: ReaderConfig,
    url: String,
    len: u64,
    validator: Option<Validator>,
    /// Bytes this reader has fetched, and the most it may fetch.
    fetched: AtomicU64,
    fetch_limit: AtomicU64,
}

/// Parses `bytes start-end/total`.
fn parse_content_range(headers: &HeaderMap) -> Result<(u64, u64, u64)> {
    let invalid = Error::InvalidResponse("missing or malformed Content-Range header");
    let value = headers
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "));
    let parsed = value.and_then(|value| {
        let (range, total) = value.split_once('/')?;
        let (start, end) = range.split_once('-')?;
        Some((
            start.trim().parse().ok()?,
            end.trim().parse().ok()?,
            total.trim().parse().ok()?,
        ))
    });
    match parsed {
        Some((start, end, total)) if start <= end && end < total => Ok((start, end, total)),
        _ => Err(invalid),
    }
}

fn check_not_encoded(headers: &HeaderMap) -> Result<()> {
    match headers.get(CONTENT_ENCODING).map(|value| value.as_bytes()) {
        None | Some(b"identity") => Ok(()),
        Some(_) => Err(Error::InvalidResponse(
            "the server compressed a partial response",
        )),
    }
}

fn is_retryable_status(status: StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 425 | 429 | 500 | 502 | 503 | 504)
}

fn is_retryable(error: &Error) -> bool {
    match error {
        Error::Http(e) => e.is_connect() || e.is_timeout() || e.is_body() || e.is_request(),
        Error::HttpStatus { status } => StatusCode::from_u16(*status)
            .map(is_retryable_status)
            .unwrap_or(false),
        Error::TruncatedResponse => true,
        _ => false,
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds: u64 = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(min(Duration::from_secs(seconds), MAX_RETRY_AFTER))
}

/// Reads a response body, stopping as soon as it's longer than `expected`.
async fn read_body(
    mut response: Response,
    expected: u64,
    config: &ReaderConfig,
) -> Result<Vec<u8>> {
    let mut body = Vec::with_capacity(expected as usize);
    loop {
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(e) if e.is_timeout() => return Err(e.into()),
            Err(_) => return Err(Error::TruncatedResponse),
        };
        config
            .network_bytes
            .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        if body.len() as u64 + chunk.len() as u64 > expected {
            return Err(Error::InvalidResponse("more data than was requested"));
        }
        body.extend_from_slice(&chunk);
    }
    if (body.len() as u64) < expected {
        return Err(Error::TruncatedResponse);
    }
    Ok(body)
}

/// Runs `attempt` under the retry policy. A failed attempt can ask for a specific wait.
async fn with_retries<T, F, Fut>(policy: &RetryPolicy, mut attempt: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<T, (Error, Option<Duration>)>>,
{
    let mut tries = 0;
    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err((error, wait)) => {
                tries += 1;
                if tries >= policy.max_attempts || !is_retryable(&error) {
                    return Err(error);
                }
                tokio::time::sleep(wait.unwrap_or_else(|| policy.delay(tries - 1))).await;
            }
        }
    }
}

impl Remote {
    /// Fetches some of `start..end`. A server may legally send less than was asked for.
    async fn fetch_range(self: Arc<Self>, start: u64, end: u64) -> Result<(u64, Bytes)> {
        let limit = self.fetch_limit.load(Ordering::Relaxed);
        if self
            .fetched
            .load(Ordering::Relaxed)
            .saturating_add(end - start)
            > limit
        {
            return Err(Error::LimitExceeded(
                "reading its directory used too much data",
            ));
        }

        let body = with_retries(&self.config.retry, || self.fetch_once(start, end)).await?;
        self.fetched.fetch_add(body.len() as u64, Ordering::Relaxed);
        Ok((start, Bytes::from(body)))
    }

    async fn fetch_once(
        &self,
        start: u64,
        end: u64,
    ) -> std::result::Result<Vec<u8>, (Error, Option<Duration>)> {
        let response = self
            .config
            .client
            .get(&self.url)
            .header(RANGE, format!("bytes={}-{}", start, end - 1))
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|e| (Error::from(e), None))?;

        let status = response.status();
        let headers = response.headers();
        if status != StatusCode::PARTIAL_CONTENT {
            if status.is_success() {
                // Dropping the response closes the connection without reading the body.
                let changed =
                    self.validator.is_some() && self.validator != Validator::from_headers(headers);
                let error = if changed {
                    Error::RemoteChanged
                } else {
                    Error::RangeNotSupported
                };
                return Err((error, None));
            }
            let wait = retry_after(headers);
            return Err((
                Error::HttpStatus {
                    status: status.as_u16(),
                },
                wait,
            ));
        }

        let checked = (|| {
            check_not_encoded(headers)?;
            if let (Some(known), Some(current)) =
                (&self.validator, Validator::from_headers(headers))
            {
                if *known != current {
                    return Err(Error::RemoteChanged);
                }
            }
            let (got_start, got_end, total) = parse_content_range(headers)?;
            if total != self.len {
                return Err(Error::RemoteChanged);
            }
            if got_start != start || got_end >= end {
                return Err(Error::InvalidResponse(
                    "the data isn't the part that was requested",
                ));
            }
            Ok(got_end - got_start + 1)
        })();
        let expected = checked.map_err(|e| (e, None))?;

        read_body(response, expected, &self.config)
            .await
            .map_err(|e| (e, None))
    }
}

/// Finds the file's length and fetches its end. Not every server that supports ranges supports a
/// suffix range (GitHub release assets answer 501), so this falls back to an explicit one.
async fn open_tail(config: &ReaderConfig, url: &str) -> Result<(u64, Bytes, Option<Validator>)> {
    let get = |range: String, passes: fn(StatusCode) -> bool| async move {
        let response = config
            .client
            .get(url)
            .header(RANGE, range)
            .header(ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|e| (Error::from(e), None))?;
        let status = response.status();
        if status.is_success() || passes(status) {
            return Ok(response);
        }
        Err((
            Error::HttpStatus {
                status: status.as_u16(),
            },
            retry_after(response.headers()),
        ))
    };

    let suffix = with_retries(&config.retry, || {
        get(format!("bytes=-{TAIL_BYTES}"), |status| {
            matches!(status.as_u16(), 400 | 416 | 501)
        })
    })
    .await?;
    let response = if suffix.status() == StatusCode::PARTIAL_CONTENT {
        suffix
    } else {
        drop(suffix);
        let probe = with_retries(&config.retry, || {
            get("bytes=0-0".to_string(), |status| {
                status == StatusCode::RANGE_NOT_SATISFIABLE
            })
        })
        .await?;
        match probe.status() {
            StatusCode::PARTIAL_CONTENT => {}
            // An empty file can't satisfy any range.
            StatusCode::RANGE_NOT_SATISFIABLE => return Err(Error::NotAZip),
            _ => return Err(Error::RangeNotSupported),
        }
        let (_, _, total) = parse_content_range(probe.headers())?;
        drop(probe);
        let start = total.saturating_sub(TAIL_BYTES);
        with_retries(&config.retry, || {
            get(format!("bytes={start}-{}", total - 1), |_| false)
        })
        .await?
    };

    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(Error::RangeNotSupported);
    }
    check_not_encoded(response.headers())?;
    let validator = Validator::from_headers(response.headers());
    let (start, end, total) = parse_content_range(response.headers())?;
    if end + 1 != total || end - start + 1 > TAIL_BYTES {
        return Err(Error::InvalidResponse(
            "the data isn't the part that was requested",
        ));
    }
    let body = read_body(response, end - start + 1, config).await?;
    Ok((total, Bytes::from(body), validator))
}

/// Lets directory parsing fetch only a bounded amount of data, until the limit is lifted.
pub struct FetchLimit(Arc<Remote>);

impl FetchLimit {
    pub fn lift(&self) {
        self.0.fetch_limit.store(u64::MAX, Ordering::Relaxed);
    }
}

pub struct RemoteHttpReader {
    remote: Arc<Remote>,
    pos: u64,
    bytes_read: u64,
    /// The end of the file, kept for the reader's whole life so the end records that were checked
    /// are always the ones parsed.
    tail: Bytes,
    tail_start: u64,
    state: Option<State>,
}

enum State {
    Idle,
    Requesting(BoxFuture<'static, Result<(u64, Bytes)>>),
    Buffered(Bytes, u64), // data, start_pos
}

impl RemoteHttpReader {
    pub async fn new(url: &str) -> Result<Self> {
        Self::open(url, ReaderConfig::new()?).await
    }

    pub async fn open(url: &str, config: ReaderConfig) -> Result<Self> {
        let (len, tail, validator) = open_tail(&config, url).await?;
        check_directory(&tail, len)?;

        let tail_start = len - tail.len() as u64;
        let remote = Remote {
            config,
            url: url.to_string(),
            len,
            validator,
            fetched: AtomicU64::new(tail.len() as u64),
            fetch_limit: AtomicU64::new(MAX_DIRECTORY_BYTES.saturating_add(TAIL_BYTES * 2)),
        };
        Ok(Self {
            remote: Arc::new(remote),
            pos: 0,
            bytes_read: 0,
            tail,
            tail_start,
            state: Some(State::Idle),
        })
    }

    /// Handle for lifting the limit on fetched data, once the directory has been parsed.
    pub fn fetch_limit(&self) -> FetchLimit {
        FetchLimit(self.remote.clone())
    }

    pub fn get_total_bytes_read(&self) -> u64 {
        self.bytes_read
    }

    /// Bytes received over the network, which includes data never read by the consumer.
    pub fn get_network_bytes(&self) -> u64 {
        self.remote.config.network_bytes.load(Ordering::Relaxed)
    }

    pub fn get_content_length(&self) -> u64 {
        self.remote.len
    }

    pub fn url(&self) -> &str {
        &self.remote.url
    }

    fn copy_from(&mut self, data: Bytes, data_start: u64, buf: &mut ReadBuf<'_>) -> bool {
        if self.pos < data_start || self.pos >= data_start + data.len() as u64 {
            return false;
        }
        let offset = (self.pos - data_start) as usize;
        let to_read = min(data.len() - offset, buf.remaining());
        buf.put_slice(&data[offset..offset + to_read]);
        self.pos += to_read as u64;
        self.bytes_read += to_read as u64;
        true
    }
}

impl AsyncSeek for RemoteHttpReader {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        let new_pos = match position {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(p) => self.remote.len.checked_add_signed(p),
            SeekFrom::Current(p) => self.pos.checked_add_signed(p),
        };
        let new_pos = new_pos.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek to a position before the start of the file",
            )
        })?;
        self.pos = min(new_pos, self.remote.len);
        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        Poll::Ready(Ok(self.pos))
    }
}

impl AsyncRead for RemoteHttpReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.pos >= self.remote.len || buf.remaining() == 0 {
            return Poll::Ready(Ok(())); // EOF
        }

        let (tail, tail_start) = (self.tail.clone(), self.tail_start);
        if self.copy_from(tail, tail_start, buf) {
            return Poll::Ready(Ok(()));
        }

        loop {
            let state = self.state.take().unwrap_or(State::Idle);
            match state {
                State::Idle => {
                    let start = self.pos;
                    let chunk_end = start.saturating_add(self.remote.config.chunk_bytes);
                    // The tail is already here, so stop where it starts when reading up to it.
                    let boundary = if start < self.tail_start {
                        self.tail_start
                    } else {
                        self.remote.len
                    };
                    let end = min(chunk_end, boundary);
                    let future = self.remote.clone().fetch_range(start, end);
                    self.state = Some(State::Requesting(Box::pin(future)));
                }
                State::Requesting(mut fut) => {
                    match fut.as_mut().poll(cx) {
                        Poll::Ready(Ok((start, data))) => {
                            self.state = Some(State::Buffered(data, start));
                        }
                        Poll::Ready(Err(e)) => {
                            self.state = Some(State::Idle); // Reset to idle on error
                            return Poll::Ready(Err(e.into()));
                        }
                        Poll::Pending => {
                            self.state = Some(State::Requesting(fut));
                            return Poll::Pending;
                        }
                    }
                }
                State::Buffered(data, start_pos) => {
                    if self.copy_from(data.clone(), start_pos, buf) {
                        self.state = Some(State::Buffered(data, start_pos));
                        return Poll::Ready(Ok(()));
                    }
                    // Buffer does not cover current pos. Discard and fetch new.
                    self.state = Some(State::Idle);
                }
            }
        }
    }
}
