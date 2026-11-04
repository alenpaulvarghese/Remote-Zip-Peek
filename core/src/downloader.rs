use crate::error::{Error, Result};
use crate::http_reader::{ReaderConfig, RemoteHttpReader};
use crate::safe_path::{destination, ensure_within};
use async_zip::base::read::seek::ZipFileReader;
use async_zip::{Compression, StoredZipEntry};
use futures::stream::{self, StreamExt};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tokio_util::sync::CancellationToken;

const COPY_BUFFER_BYTES: usize = 64 * 1024;
const WRITE_BUFFER_BYTES: usize = 1024 * 1024;
const EXISTENCE_CHECK_CONCURRENCY: usize = 32;
/// Deflate can't compress better than about 1032:1, so a higher claimed ratio is a lie.
const MAX_COMPRESSION_RATIO: u64 = 1100;
const RATIO_CHECK_MIN_BYTES: u64 = 1024 * 1024;

/// What an entry's directory record promises, checked as its data is read. This keeps a zip bomb
/// from writing more than the size shown to the user.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EntryCheck {
    size: u64,
    crc: u32,
}

impl EntryCheck {
    pub(crate) fn new(entry: &StoredZipEntry, path: &str) -> Result<Self> {
        let size = entry.uncompressed_size();
        let compressed = entry.compressed_size();
        let suspicious = |reason| Error::SuspiciousEntry {
            path: path.to_string(),
            reason,
        };
        // ZIP's LZMA framing differs from what the decoder expects, so it would only ever fail.
        if entry.compression() == Compression::Lzma {
            return Err(Error::UnsupportedCompression {
                path: path.to_string(),
                method: "LZMA",
            });
        }
        if entry.compression() == Compression::Stored && size != compressed {
            return Err(suspicious("an uncompressed file has two different sizes"));
        }
        if size >= RATIO_CHECK_MIN_BYTES && size / compressed.max(1) > MAX_COMPRESSION_RATIO {
            return Err(suspicious("it claims an impossible compression ratio"));
        }
        Ok(Self {
            size,
            crc: entry.crc32(),
        })
    }

    /// Reading one byte more than promised is enough to detect a larger entry.
    pub(crate) fn read_limit(&self) -> u64 {
        self.size.saturating_add(1)
    }

    pub(crate) fn check_size_so_far(&self, total: u64, path: &str) -> Result<()> {
        if total > self.size {
            return Err(Error::SizeMismatch {
                path: path.to_string(),
                expected: self.size,
                actual: total,
            });
        }
        Ok(())
    }

    pub(crate) fn check_finished(&self, total: u64, crc: u32, path: &str) -> Result<()> {
        if total != self.size {
            return Err(Error::SizeMismatch {
                path: path.to_string(),
                expected: self.size,
                actual: total,
            });
        }
        if crc != self.crc {
            return Err(Error::CrcMismatch {
                path: path.to_string(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BatchFile {
    pub zip_path: String,
    pub dest: PathBuf,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FailedFile {
    pub path: String,
    pub error: String,
}

/// Where each selected file will be written, and the files that can't be.
#[derive(Debug, Default)]
pub struct SelectionPlan {
    pub files: Vec<BatchFile>,
    pub rejected: Vec<FailedFile>,
}

/// Maps selected entries to files under `root`, relative to `base` in the archive.
///
/// Names that differ only by case are rejected, since they collide on the default macOS and
/// Windows file systems.
pub fn plan_selection(root: &Path, base: &str, zip_paths: Vec<String>) -> SelectionPlan {
    let mut plan = SelectionPlan::default();
    let mut seen = HashSet::with_capacity(zip_paths.len());

    for zip_path in zip_paths {
        match destination(root, base, &zip_path) {
            Ok(dest) if seen.insert(dest.to_string_lossy().to_lowercase()) => {
                plan.files.push(BatchFile { zip_path, dest });
            }
            Ok(_) => plan.rejected.push(FailedFile {
                path: zip_path,
                error: "Another selected file would be written to the same path".to_string(),
            }),
            Err(error) => plan.rejected.push(FailedFile {
                path: zip_path,
                error: error.to_string(),
            }),
        }
    }
    plan
}

/// Returns the entries whose destination already exists.
pub async fn find_existing(files: &[BatchFile]) -> Vec<String> {
    stream::iter(files.to_vec())
        .map(|file| async move {
            matches!(tokio::fs::try_exists(&file.dest).await, Ok(true)).then_some(file.zip_path)
        })
        .buffer_unordered(EXISTENCE_CHECK_CONCURRENCY)
        .filter_map(|existing| async move { existing })
        .collect()
        .await
}

#[derive(Debug, PartialEq)]
pub enum BatchEvent<'a> {
    FileStarted { path: &'a str },
    FileBytes { path: &'a str, bytes: u64 },
    FileFinished { path: &'a str },
    FileFailed { path: &'a str, error: String },
}

pub struct BatchOptions {
    /// Number of files downloaded at once. Zero is treated as one.
    pub concurrency: usize,
    pub cancel: CancellationToken,
    /// When set, destination folders must resolve inside this folder.
    pub root: Option<PathBuf>,
    pub reader: ReaderConfig,
}

#[derive(Serialize, Debug, Default, PartialEq)]
pub struct BatchReport {
    pub completed: u32,
    pub failed: u32,
    pub cancelled: bool,
    pub written_bytes: u64,
    pub network_bytes: u64,
}

/// Removes the file when dropped, unless it has been renamed into place.
struct TempFileGuard(Option<PathBuf>);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn temp_path_for(dest: &Path) -> PathBuf {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    dest.with_file_name(format!(".lazy-zip-{}-{id}.part", std::process::id()))
}

/// Records where a file came from, so Windows treats it like a browser download. On macOS the
/// app's Info.plist asks the system to quarantine files it creates instead.
#[cfg(windows)]
fn mark_as_downloaded(dest: &Path, url: &str) {
    // The query string is left out, because it can contain a signed token.
    let host_url = url.split(['?', '#']).next().unwrap_or("");
    let mut stream = dest.as_os_str().to_os_string();
    stream.push(":Zone.Identifier");
    let contents = format!("[ZoneTransfer]\r\nZoneId=3\r\nHostUrl={host_url}\r\n");
    let _ = std::fs::write(stream, contents);
}

#[cfg(not(windows))]
fn mark_as_downloaded(_dest: &Path, _url: &str) {}

/// Downloads one entry to a temporary file, then renames it over `dest`, so a failed or cancelled
/// download never replaces an existing file or leaves a partial one behind.
async fn download_entry(
    url: &str,
    file: &BatchFile,
    root: Option<&Path>,
    config: ReaderConfig,
    on_bytes: &(dyn Fn(u64) + Send + Sync),
) -> Result<u64> {
    if let Some(parent) = file.dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
        if let Some(root) = root {
            ensure_within(root, parent)?;
        }
    }

    let reader = RemoteHttpReader::open(url, config).await?;
    let fetch_limit = reader.fetch_limit();
    let mut zip = ZipFileReader::new(BufReader::new(reader).compat()).await?;
    fetch_limit.lift();

    // The last duplicate wins, matching what the listing shows.
    let index = zip
        .file()
        .entries()
        .iter()
        .rposition(|e| e.filename().as_str().unwrap_or("") == file.zip_path)
        .ok_or_else(|| Error::FileNotFound(file.zip_path.clone()))?;
    let check = EntryCheck::new(&zip.file().entries()[index], &file.zip_path)?;

    let entry_reader = zip.reader_without_entry(index).await?;
    let mut entry_reader = entry_reader.compat();
    let mut limited = (&mut entry_reader).take(check.read_limit());

    let temp_path = temp_path_for(&file.dest);
    let mut guard = TempFileGuard(None);
    let temp_file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .await?;
    guard.0 = Some(temp_path.clone());
    let mut writer = BufWriter::with_capacity(WRITE_BUFFER_BYTES, temp_file);

    let mut buffer = vec![0u8; COPY_BUFFER_BYTES];
    let mut written = 0u64;
    loop {
        let n = limited.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        written += n as u64;
        check.check_size_so_far(written, &file.zip_path)?;
        writer.write_all(&buffer[..n]).await?;
        on_bytes(n as u64);
    }
    check.check_finished(
        written,
        entry_reader.get_mut().compute_hash(),
        &file.zip_path,
    )?;
    writer.shutdown().await?;
    drop(writer);

    tokio::fs::rename(&temp_path, &file.dest).await?;
    guard.0 = None;
    mark_as_downloaded(&file.dest, url);
    Ok(written)
}

/// Downloads files concurrently, carrying on past individual failures.
///
/// Each file independently opens its own HTTP reader. Every outcome is reported through
/// `on_event` as it happens, and cancelling stops work and removes partial files.
pub async fn download_batch<F>(
    url: &str,
    files: Vec<BatchFile>,
    options: BatchOptions,
    on_event: F,
) -> BatchReport
where
    F: for<'a> Fn(BatchEvent<'a>) + Send + Sync,
{
    let completed = AtomicU32::new(0);
    let failed = AtomicU32::new(0);
    let written_bytes = AtomicU64::new(0);

    let downloads = files.into_iter().map(|file| {
        let (cancel, root) = (&options.cancel, options.root.as_deref());
        let (completed, failed, written_bytes) = (&completed, &failed, &written_bytes);
        let config = options.reader.clone();
        let on_event = &on_event;
        async move {
            if cancel.is_cancelled() {
                return;
            }
            let path = file.zip_path.as_str();
            on_event(BatchEvent::FileStarted { path });
            let on_bytes = |bytes| on_event(BatchEvent::FileBytes { path, bytes });
            let result = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(Error::Cancelled),
                result = download_entry(url, &file, root, config, &on_bytes) => result,
            };
            match result {
                Ok(written) => {
                    written_bytes.fetch_add(written, Ordering::Relaxed);
                    completed.fetch_add(1, Ordering::Relaxed);
                    on_event(BatchEvent::FileFinished { path });
                }
                Err(Error::Cancelled) => {}
                Err(error) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    on_event(BatchEvent::FileFailed {
                        path,
                        error: error.to_string(),
                    });
                }
            }
        }
    });

    stream::iter(downloads)
        .buffer_unordered(options.concurrency.max(1))
        .collect::<Vec<()>>()
        .await;

    BatchReport {
        completed: completed.load(Ordering::Relaxed),
        failed: failed.load(Ordering::Relaxed),
        cancelled: options.cancel.is_cancelled(),
        written_bytes: written_bytes.load(Ordering::Relaxed),
        network_bytes: options.reader.network_bytes.load(Ordering::Relaxed),
    }
}
