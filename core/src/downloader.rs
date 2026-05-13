use crate::error::{Error, Result};
use crate::http_reader::RemoteHttpReader;
use async_zip::base::read::seek::ZipFileReader;
use futures::stream::{self, StreamExt};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::io::BufReader;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

#[derive(Serialize, Clone, Debug)]
pub struct DownloadProgress {
    pub current_file: String,
    pub completed: u32,
    pub total: u32,
}

/// Download a single file from a remote ZIP archive directly to a local path.
pub async fn download_file_to_path(url: &str, zip_path: &str, dest: &Path) -> Result<u64> {
    let reader = RemoteHttpReader::new(url).await?;
    let mut zip = ZipFileReader::new(BufReader::new(reader).compat()).await?;

    let index = zip
        .file()
        .entries()
        .iter()
        .position(|e| e.filename().as_str().unwrap_or("") == zip_path)
        .ok_or_else(|| Error::FileNotFound(zip_path.to_string()))?;

    let entry_reader = zip
        .reader_without_entry(index)
        .await
        .map_err(|e| Error::Zip(e.to_string()))?;

    let mut compat_reader = entry_reader.compat();
    let mut file = tokio::fs::File::create(dest).await?;
    let bytes_written = tokio::io::copy(&mut compat_reader, &mut file).await?;

    Ok(bytes_written)
}

/// Download multiple files concurrently from a remote ZIP archive.
///
/// Spawns up to `concurrency` parallel workers. Each worker independently
/// opens its own HTTP reader and extracts one file at a time.
/// Progress is reported atomically as each file completes.
pub async fn download_batch<F>(
    url: &str,
    files: Vec<(String, PathBuf)>,
    concurrency: usize,
    on_progress: F,
) -> Result<u32>
where
    F: Fn(DownloadProgress) + Send + Sync,
{
    let total = files.len() as u32;
    if total == 0 {
        return Ok(0);
    }

    // Pre-create all parent directories
    for (_, dest_path) in &files {
        if let Some(parent) = dest_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    let completed = Arc::new(AtomicU32::new(0));
    let on_progress = Arc::new(on_progress);
    let url = url.to_string();

    let results: Vec<Result<()>> = stream::iter(files)
        .map(|(zip_path, dest_path)| {
            let u = url.clone();
            let c = completed.clone();
            let p = on_progress.clone();
            async move {
                download_file_to_path(&u, &zip_path, &dest_path).await?;
                let done = c.fetch_add(1, Ordering::SeqCst) + 1;
                p(DownloadProgress {
                    current_file: zip_path,
                    completed: done,
                    total,
                });
                Ok(())
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await;

    // Return first error if any
    for r in results {
        r?;
    }

    Ok(completed.load(Ordering::SeqCst))
}
