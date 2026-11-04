//! A local HTTP server that can misbehave on purpose, and ZIP builders for tests.
#![allow(dead_code)]

use lazy_zip_core::http_reader::{ReaderConfig, RetryPolicy};
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub contents: Vec<u8>,
    pub deflate: bool,
    /// Overrides the checksum and uncompressed size in the directory, to simulate corruption.
    pub wrong_crc: bool,
    pub declared_size: Option<u32>,
}

pub fn stored(name: &str, contents: impl Into<Vec<u8>>) -> Entry {
    Entry {
        name: name.to_string(),
        contents: contents.into(),
        deflate: false,
        wrong_crc: false,
        declared_size: None,
    }
}

pub fn deflated(name: &str, contents: impl Into<Vec<u8>>) -> Entry {
    Entry {
        deflate: true,
        ..stored(name, contents)
    }
}

pub fn build_zip(entries: &[Entry]) -> Vec<u8> {
    build_zip_with_comment(entries, b"")
}

pub fn build_zip_with_comment(entries: &[Entry], comment: &[u8]) -> Vec<u8> {
    let mut data = Vec::new();
    let mut directory = Vec::new();
    for entry in entries {
        let offset = data.len() as u32;
        let crc = crc32fast::hash(&entry.contents) ^ u32::from(entry.wrong_crc);
        let compressed = if entry.deflate {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&entry.contents).unwrap();
            encoder.finish().unwrap()
        } else {
            entry.contents.clone()
        };
        let size = entry.declared_size.unwrap_or(entry.contents.len() as u32);
        let method: u16 = if entry.deflate { 8 } else { 0 };
        let common = |out: &mut Vec<u8>| {
            out.extend_from_slice(&20u16.to_le_bytes()); // version needed
            out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 4]); // modification time
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0; 2]); // extra length
        };

        data.extend_from_slice(b"PK\x03\x04");
        common(&mut data);
        data.extend_from_slice(entry.name.as_bytes());
        data.extend_from_slice(&compressed);

        directory.extend_from_slice(b"PK\x01\x02");
        directory.extend_from_slice(&20u16.to_le_bytes()); // version made by
        common(&mut directory);
        directory.extend_from_slice(&[0; 10]); // comment, disk, internal and external attributes
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(entry.name.as_bytes());
    }

    let directory_offset = data.len() as u32;
    data.extend_from_slice(&directory);
    data.extend_from_slice(b"PK\x05\x06");
    data.extend_from_slice(&[0; 4]); // disk numbers
    data.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    data.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    data.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    data.extend_from_slice(&directory_offset.to_le_bytes());
    data.extend_from_slice(&(comment.len() as u16).to_le_bytes());
    data.extend_from_slice(comment);
    data
}

#[derive(Clone, Default)]
pub struct Behavior {
    /// Reply 200 with the whole file to range requests.
    pub ignore_range: bool,
    /// Reply 405 to HEAD. The client shouldn't send any.
    pub reject_head: bool,
    /// Reply 200 to suffix ranges (`bytes=-N`) but honor explicit ones.
    pub no_suffix_ranges: bool,
    /// Reply with this status to suffix ranges but honor explicit ones, as GitHub does with 501.
    pub suffix_range_status: Option<u16>,
    /// Report and send a range this many bytes later than requested.
    pub shift_range: i64,
    /// Close the connection after this many body bytes, having promised the full range.
    pub truncate_after: Option<usize>,
    /// Reply 206 with an empty body.
    pub empty_body: bool,
    /// Send this many extra bytes beyond the requested range, claiming them in Content-Length.
    pub extra_body_bytes: usize,
    /// Mark responses as compressed.
    pub content_encoding: Option<&'static str>,
    /// Reply with this status to the first N GET requests.
    pub fail_first: Option<(u32, u16, Option<u32>)>,
    /// Reply with this status to every GET request after the first N.
    pub fail_after: Option<(u32, u16)>,
    /// Keep the connection open without replying, after the first N GET requests.
    pub stall_after: Option<u32>,
    /// Pause before answering each GET request.
    pub get_delay: Option<Duration>,
    /// Send an ETag, which changes after the first N GET requests.
    pub etag: Option<&'static str>,
    pub change_etag_after: Option<u32>,
    /// Report a different total file size after the first N GET requests.
    pub change_length_after: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub range: Option<String>,
    pub user_agent: Option<String>,
    pub status: u16,
    pub body_bytes: usize,
}

pub struct Server {
    pub url: String,
    log: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }

    pub fn gets(&self) -> usize {
        self.requests().iter().filter(|r| r.method == "GET").count()
    }

    /// Body bytes the server tried to send.
    pub fn body_bytes(&self) -> usize {
        self.requests().iter().map(|r| r.body_bytes).sum()
    }
}

pub async fn serve(archive: Vec<u8>) -> Server {
    serve_with(archive, Behavior::default()).await
}

pub async fn serve_with(archive: Vec<u8>, behavior: Behavior) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/test.zip", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::new((archive, behavior, AtomicU32::new(0), log.clone()));

    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let shared = shared.clone();
            tokio::spawn(async move {
                let (archive, behavior, gets, log) = &*shared;
                let mut raw = Vec::new();
                let mut chunk = [0u8; 4096];
                while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => raw.extend_from_slice(&chunk[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&raw).to_string();
                let header = |name: &str| {
                    text.lines().find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case(name)
                            .then(|| value.trim().to_string())
                    })
                };
                let mut record = Request {
                    method: text.split(' ').next().unwrap_or("").to_string(),
                    range: header("range"),
                    user_agent: header("user-agent"),
                    status: 0,
                    body_bytes: 0,
                };
                let (status, head, body) = respond(&record, archive, behavior, gets).await;
                record.status = status;
                record.body_bytes = body.len();
                log.lock().unwrap().push(record);

                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Server { url, log }
}

fn reply(status: u16, reason: &str, headers: &str, length: usize) -> String {
    format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {length}\r\n{headers}Connection: close\r\n\r\n")
}

async fn respond(
    request: &Request,
    archive: &[u8],
    behavior: &Behavior,
    gets: &AtomicU32,
) -> (u16, String, Vec<u8>) {
    if request.method == "HEAD" {
        return if behavior.reject_head {
            (405, reply(405, "Method Not Allowed", "", 0), Vec::new())
        } else {
            (200, reply(200, "OK", "", archive.len()), Vec::new())
        };
    }

    if let Some(delay) = behavior.get_delay {
        tokio::time::sleep(delay).await;
    }
    let served = gets.fetch_add(1, Ordering::SeqCst);

    if behavior.stall_after.is_some_and(|limit| served >= limit) {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
    if let Some((times, status, retry_after)) = behavior.fail_first {
        if served < times {
            let headers = retry_after
                .map(|seconds| format!("Retry-After: {seconds}\r\n"))
                .unwrap_or_default();
            return (status, reply(status, "Failed", &headers, 0), Vec::new());
        }
    }
    if let Some((limit, status)) = behavior.fail_after {
        if served >= limit {
            return (status, reply(status, "Failed", "", 0), Vec::new());
        }
    }
    if behavior.ignore_range {
        return (200, reply(200, "OK", "", archive.len()), archive.to_vec());
    }

    let range = request.range.as_deref().unwrap_or("");
    let spec = range.strip_prefix("bytes=").unwrap_or("");
    let len = archive.len();
    let (start, end) = if let Some(suffix) = spec.strip_prefix('-') {
        if behavior.no_suffix_ranges {
            return (200, reply(200, "OK", "", len), archive.to_vec());
        }
        if let Some(status) = behavior.suffix_range_status {
            return (status, reply(status, "Unsupported", "", 0), Vec::new());
        }
        if len == 0 {
            return (416, reply(416, "Range Not Satisfiable", "", 0), Vec::new());
        }
        (len.saturating_sub(suffix.parse().unwrap()), len - 1)
    } else {
        let (start, end) = spec.split_once('-').unwrap();
        let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
        if start >= len {
            return (416, reply(416, "Range Not Satisfiable", "", 0), Vec::new());
        }
        (start, end.min(len - 1))
    };

    let shifted_start = (start as i64 + behavior.shift_range).clamp(0, len as i64 - 1) as usize;
    let shifted_end = (end as i64 + behavior.shift_range).clamp(0, len as i64 - 1) as usize;
    let mut body = archive[shifted_start..=shifted_end].to_vec();

    let reported_total = match behavior.change_length_after {
        Some(limit) if served >= limit => len + 1,
        _ => len,
    };
    let mut headers =
        format!("Content-Range: bytes {shifted_start}-{shifted_end}/{reported_total}\r\n");
    if let Some(etag) = behavior.etag {
        let changed = behavior
            .change_etag_after
            .is_some_and(|limit| served >= limit);
        let value = if changed { "\"changed\"" } else { etag };
        headers.push_str(&format!("ETag: {value}\r\n"));
    }
    if let Some(encoding) = behavior.content_encoding {
        headers.push_str(&format!("Content-Encoding: {encoding}\r\n"));
    }

    let promised = if behavior.empty_body {
        body.clear();
        0
    } else {
        body.extend(std::iter::repeat_n(0u8, behavior.extra_body_bytes));
        body.len()
    };
    if let Some(keep) = behavior.truncate_after {
        body.truncate(keep);
    }
    (206, reply(206, "Partial Content", &headers, promised), body)
}

/// A reader config that fails fast, so tests of error handling don't wait on real backoff.
pub fn fast_config() -> ReaderConfig {
    let mut config =
        ReaderConfig::with_timeouts(Duration::from_secs(2), Duration::from_secs(2)).unwrap();
    config.retry = RetryPolicy {
        max_attempts: 3,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(5),
    };
    config
}
