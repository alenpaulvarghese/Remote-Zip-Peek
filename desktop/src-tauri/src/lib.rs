use lazy_zip_core::downloader;
use lazy_zip_core::http_reader::RemoteHttpReader;
use lazy_zip_core::zip_explorer::{ScanResult, ZipExplorer};
use std::path::PathBuf;
use tauri::Emitter;

#[tauri::command]
async fn scan_zip(url: String) -> Result<ScanResult, String> {
    let reader = RemoteHttpReader::new(&url)
        .await
        .map_err(|e| e.to_string())?;
    let mut explorer = ZipExplorer::new(reader);
    explorer.list_files().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn download_file(url: String, file_path: String, save_path: String) -> Result<u64, String> {
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("zip-dl".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let result = (|| -> Result<u64, String> {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("Failed to create tokio runtime: {e}"))?;
                rt.block_on(async {
                    downloader::download_file_to_path(&url, &file_path, &PathBuf::from(&save_path))
                        .await
                        .map_err(|e| e.to_string())
                })
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("Thread spawn failed: {e}"))?;

    rx.recv()
        .map_err(|e| format!("Download thread died: {e}"))
        .and_then(|r| r)
}

#[tauri::command]
async fn download_folder(
    app: tauri::AppHandle,
    url: String,
    folder_path: String,
    save_dir: String,
    file_paths: Vec<String>,
    workers: usize,
) -> Result<u32, String> {
    let concurrency = workers.clamp(1, 50);

    let files: Vec<(String, PathBuf)> = file_paths
        .into_iter()
        .map(|fp| {
            let relative = fp
                .strip_prefix(&folder_path)
                .unwrap_or(&fp)
                .trim_start_matches('/')
                .to_string();
            let dest = PathBuf::from(&save_dir).join(&relative);
            (fp, dest)
        })
        .collect();

    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::Builder::new()
        .name("zip-dl-batch".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let result = (|| -> Result<u32, String> {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(concurrency)
                    .enable_all()
                    .thread_stack_size(8 * 1024 * 1024)
                    .build()
                    .map_err(|e| format!("Failed to create tokio runtime: {e}"))?;
                rt.block_on(async {
                    downloader::download_batch(&url, files, concurrency, |progress| {
                        let _ = app.emit("download-progress", &progress);
                    })
                    .await
                    .map_err(|e| e.to_string())
                })
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("Thread spawn failed: {e}"))?;

    rx.recv()
        .map_err(|e| format!("Download thread died: {e}"))
        .and_then(|r| r)
}

#[tauri::command]
async fn file_exists(path: String) -> bool {
    tokio::fs::metadata(&path).await.is_ok()
}

/// Given a list of local file paths, return the ones that already exist.
#[tauri::command]
async fn check_existing_files(paths: Vec<String>) -> Vec<String> {
    let mut existing = Vec::new();
    for p in paths {
        if tokio::fs::metadata(&p).await.is_ok() {
            existing.push(p);
        }
    }
    existing
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            scan_zip,
            download_file,
            download_folder,
            file_exists,
            check_existing_files
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
