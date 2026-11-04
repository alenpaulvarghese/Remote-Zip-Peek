use lazy_zip_core::downloader::{
    download_batch, find_existing, plan_selection, BatchEvent, BatchFile, BatchOptions,
    SelectionPlan,
};
use lazy_zip_core::http_reader::{ReaderConfig, RemoteHttpReader};
use lazy_zip_core::zip_explorer::{ScanResult, ZipExplorer};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::State;
use tokio_util::sync::CancellationToken;

const MAX_WORKERS: usize = 20;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const MAX_REPORTED_ACTIVE_FILES: usize = 8;
const QUIT_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

/// Tracks running downloads so they can be cancelled.
#[derive(Clone, Default)]
struct DownloadJobs {
    next_id: Arc<AtomicU64>,
    running: Arc<Mutex<HashMap<u64, CancellationToken>>>,
}

impl DownloadJobs {
    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<u64, CancellationToken>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Cancels everything and waits briefly, so quitting doesn't leave partial files behind.
    fn cancel_all_and_wait(&self, timeout: Duration) {
        for cancel in self.running().values() {
            cancel.cancel();
        }
        let started = Instant::now();
        while !self.running().is_empty() && started.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

#[derive(Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum DownloadTarget {
    /// One file, saved to the path the user picked in the save dialog.
    File { zip_path: String, dest: PathBuf },
    /// Selected files, written under `dest_dir` relative to `base` in the archive.
    Selection {
        dest_dir: PathBuf,
        base: String,
        files: Vec<String>,
    },
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ActiveFile {
    path: String,
    done: u64,
}

#[derive(Serialize, Clone)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum JobEvent {
    Progress {
        done_bytes: u64,
        files_done: u32,
        active: Vec<ActiveFile>,
    },
    FileFailed {
        path: String,
        error: String,
    },
    Finished {
        completed: u32,
        failed: u32,
        cancelled: bool,
        written_bytes: u64,
        network_bytes: u64,
    },
}

/// Turns per-file events into progress snapshots sent at a limited rate.
struct ProgressTracker {
    channel: Channel<JobEvent>,
    state: Mutex<ProgressState>,
}

struct ProgressState {
    finished_bytes: u64,
    files_done: u32,
    active: HashMap<String, u64>,
    last_sent: Instant,
}

impl ProgressTracker {
    fn new(channel: Channel<JobEvent>) -> Self {
        Self {
            channel,
            state: Mutex::new(ProgressState {
                finished_bytes: 0,
                files_done: 0,
                active: HashMap::new(),
                last_sent: Instant::now() - PROGRESS_INTERVAL,
            }),
        }
    }

    fn handle(&self, event: BatchEvent<'_>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut force_send = false;
        match event {
            BatchEvent::FileStarted { path } => {
                state.active.insert(path.to_string(), 0);
            }
            BatchEvent::FileBytes { path, bytes } => {
                if let Some(done) = state.active.get_mut(path) {
                    *done += bytes;
                }
            }
            BatchEvent::FileFinished { path } => {
                let done = state.active.remove(path).unwrap_or(0);
                state.finished_bytes += done;
                state.files_done += 1;
            }
            BatchEvent::FileFailed { path, error } => {
                state.active.remove(path);
                force_send = true;
                let _ = self.channel.send(JobEvent::FileFailed {
                    path: path.to_string(),
                    error,
                });
            }
        }

        if force_send || state.last_sent.elapsed() >= PROGRESS_INTERVAL {
            state.last_sent = Instant::now();
            let _ = self.channel.send(snapshot(&state));
        }
    }

    fn send_final_progress(&self) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.channel.send(snapshot(&state));
    }
}

fn snapshot(state: &ProgressState) -> JobEvent {
    let active_bytes: u64 = state.active.values().sum();
    let mut active: Vec<ActiveFile> = state
        .active
        .iter()
        .map(|(path, done)| ActiveFile {
            path: path.clone(),
            done: *done,
        })
        .collect();
    active.sort_by(|a, b| a.path.cmp(&b.path));
    active.truncate(MAX_REPORTED_ACTIVE_FILES);
    JobEvent::Progress {
        done_bytes: state.finished_bytes + active_bytes,
        files_done: state.files_done,
        active,
    }
}

#[tauri::command]
async fn scan_zip(url: String) -> Result<ScanResult, String> {
    let reader = RemoteHttpReader::new(&url)
        .await
        .map_err(|e| e.to_string())?;
    let mut explorer = ZipExplorer::new(reader);
    explorer.list_files().await.map_err(|e| e.to_string())
}

/// Returns the selected entries whose destination already exists.
#[tauri::command]
async fn find_existing_files(dest_dir: PathBuf, base: String, files: Vec<String>) -> Vec<String> {
    find_existing(&plan_selection(&dest_dir, &base, files).files).await
}

/// Starts a download and returns its job ID. Progress arrives through `on_event`.
#[tauri::command]
fn start_download(
    jobs: State<'_, DownloadJobs>,
    url: String,
    target: DownloadTarget,
    workers: usize,
    on_event: Channel<JobEvent>,
) -> Result<u64, String> {
    let reader = ReaderConfig::new().map_err(|e| e.to_string())?;
    let (plan, root) = match target {
        DownloadTarget::File { zip_path, dest } => (
            SelectionPlan {
                files: vec![BatchFile { zip_path, dest }],
                rejected: Vec::new(),
            },
            None,
        ),
        DownloadTarget::Selection {
            dest_dir,
            base,
            files,
        } => (plan_selection(&dest_dir, &base, files), Some(dest_dir)),
    };

    let cancel = CancellationToken::new();
    let job_id = jobs.next_id.fetch_add(1, Ordering::Relaxed);
    jobs.running().insert(job_id, cancel.clone());

    let jobs = jobs.inner().clone();
    tauri::async_runtime::spawn(async move {
        let rejected_count = plan.rejected.len() as u32;
        for rejected in plan.rejected {
            let _ = on_event.send(JobEvent::FileFailed {
                path: rejected.path,
                error: rejected.error,
            });
        }

        let tracker = ProgressTracker::new(on_event.clone());
        let options = BatchOptions {
            concurrency: workers.clamp(1, MAX_WORKERS),
            cancel,
            root,
            reader,
        };
        let report = download_batch(&url, plan.files, options, |event| tracker.handle(event)).await;

        jobs.running().remove(&job_id);
        tracker.send_final_progress();
        let _ = on_event.send(JobEvent::Finished {
            completed: report.completed,
            failed: report.failed + rejected_count,
            cancelled: report.cancelled,
            written_bytes: report.written_bytes,
            network_bytes: report.network_bytes,
        });
    });

    Ok(job_id)
}

#[tauri::command]
fn cancel_download(jobs: State<'_, DownloadJobs>, job_id: u64) {
    if let Some(cancel) = jobs.running().get(&job_id) {
        cancel.cancel();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let jobs = DownloadJobs::default();
    tauri::Builder::default()
        .manage(jobs.clone())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            scan_zip,
            find_existing_files,
            start_download,
            cancel_download
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(move |_, event| {
            if let tauri::RunEvent::Exit = event {
                jobs.cancel_all_and_wait(QUIT_CLEANUP_TIMEOUT);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;
    use tauri::ipc::InvokeResponseBody;

    #[test]
    fn download_targets_match_what_the_frontend_sends() {
        let selection: DownloadTarget = serde_json::from_value(json!({
            "kind": "selection", "destDir": "/out", "base": "data", "files": ["data/a.txt"],
        }))
        .unwrap();
        assert!(matches!(
            selection,
            DownloadTarget::Selection { dest_dir, base, files }
                if dest_dir == Path::new("/out") && base == "data" && files == ["data/a.txt"]
        ));

        let file: DownloadTarget = serde_json::from_value(json!({
            "kind": "file", "zipPath": "a.txt", "dest": "/out/a.txt",
        }))
        .unwrap();
        assert!(matches!(
            file,
            DownloadTarget::File { zip_path, dest }
                if zip_path == "a.txt" && dest == Path::new("/out/a.txt")
        ));
    }

    #[test]
    fn job_events_match_what_the_frontend_expects() {
        let finished = JobEvent::Finished {
            completed: 1,
            failed: 2,
            cancelled: false,
            written_bytes: 3,
            network_bytes: 4,
        };
        assert_eq!(
            serde_json::to_value(finished).unwrap(),
            json!({
                "kind": "finished", "completed": 1, "failed": 2, "cancelled": false,
                "writtenBytes": 3, "networkBytes": 4,
            })
        );

        let failed = JobEvent::FileFailed {
            path: "a".to_string(),
            error: "b".to_string(),
        };
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            json!({ "kind": "fileFailed", "path": "a", "error": "b" })
        );
    }

    fn collecting_tracker() -> (ProgressTracker, Arc<Mutex<Vec<serde_json::Value>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = events.clone();
        let channel = Channel::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                collected.lock().unwrap().push(serde_json::from_str(&text)?);
            }
            Ok(())
        });
        (ProgressTracker::new(channel), events)
    }

    #[test]
    fn progress_counts_active_and_finished_files_but_not_failed_ones() {
        let (tracker, events) = collecting_tracker();
        for path in ["a", "b", "c"] {
            tracker.handle(BatchEvent::FileStarted { path });
        }
        tracker.handle(BatchEvent::FileBytes {
            path: "a",
            bytes: 10,
        });
        tracker.handle(BatchEvent::FileBytes {
            path: "b",
            bytes: 20,
        });
        tracker.handle(BatchEvent::FileBytes {
            path: "c",
            bytes: 40,
        });
        tracker.handle(BatchEvent::FileFinished { path: "a" });
        tracker.handle(BatchEvent::FileFailed {
            path: "b",
            error: "boom".to_string(),
        });
        tracker.send_final_progress();

        let events = events.lock().unwrap();
        assert!(events.iter().any(|event| event["kind"] == "fileFailed"));
        let last = events.last().unwrap();
        assert_eq!(last["kind"], "progress");
        assert_eq!(last["doneBytes"], 50);
        assert_eq!(last["filesDone"], 1);
        assert_eq!(last["active"], json!([{ "path": "c", "done": 40 }]));
    }

    #[test]
    fn quitting_cancels_running_jobs_and_waits_for_them() {
        let jobs = DownloadJobs::default();
        let cancel = CancellationToken::new();
        jobs.running().insert(1, cancel.clone());
        let background = jobs.clone();
        let worker = std::thread::spawn(move || {
            while !cancel.is_cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            background.running().remove(&1);
        });

        jobs.cancel_all_and_wait(Duration::from_secs(5));

        assert!(jobs.running().is_empty());
        worker.join().unwrap();
    }

    #[test]
    fn quitting_gives_up_waiting_after_the_timeout() {
        let jobs = DownloadJobs::default();
        jobs.running().insert(1, CancellationToken::new());
        let started = Instant::now();
        jobs.cancel_all_and_wait(Duration::from_millis(50));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn progress_is_rate_limited() {
        let (tracker, events) = collecting_tracker();
        tracker.handle(BatchEvent::FileStarted { path: "a" });
        for _ in 0..10_000 {
            tracker.handle(BatchEvent::FileBytes {
                path: "a",
                bytes: 1,
            });
        }
        assert!(events.lock().unwrap().len() < 100);
    }
}
