//! End-to-end tests of scanning and downloading against a local HTTP server with Range support.

mod support;

use lazy_zip_core::downloader::{
    download_batch, find_existing, plan_selection, BatchEvent, BatchFile, BatchOptions, BatchReport,
};
use lazy_zip_core::http_reader::RemoteHttpReader;
use lazy_zip_core::zip_explorer::{ScanResult, ZipExplorer};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use support::*;
use tokio_util::sync::CancellationToken;

async fn scan(url: &str) -> ScanResult {
    let reader = RemoteHttpReader::open(url, fast_config()).await.unwrap();
    ZipExplorer::new(reader).list_files().await.unwrap()
}

fn options(root: Option<&Path>) -> BatchOptions {
    BatchOptions {
        concurrency: 4,
        cancel: CancellationToken::new(),
        root: root.map(Path::to_path_buf),
        reader: fast_config(),
    }
}

async fn download_selection(
    url: &str,
    root: &Path,
    base: &str,
    files: &[&str],
) -> (BatchReport, Vec<(String, String)>) {
    let plan = plan_selection(
        root,
        base,
        files.iter().map(|file| file.to_string()).collect(),
    );
    assert!(plan.rejected.is_empty(), "{:?}", plan.rejected);
    let failures = Mutex::new(Vec::new());
    let report = download_batch(url, plan.files, options(Some(root)), |event| {
        if let BatchEvent::FileFailed { path, error } = event {
            failures.lock().unwrap().push((path.to_string(), error));
        }
    })
    .await;
    (report, failures.into_inner().unwrap())
}

/// Lists everything in `dir`, including hidden temporary files.
fn dir_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn scan_lists_safe_entries_and_reports_unsafe_ones() {
    let url = serve(build_zip(&[
        stored("docs/readme.txt", "hello"),
        stored("a/b/../../../evil.txt", "nope"),
    ]))
    .await
    .url;

    let result = scan(&url).await;
    let paths: Vec<_> = result.nodes.iter().map(|n| n.path.as_str()).collect();
    assert_eq!(paths, ["docs", "docs/readme.txt"]);
    assert_eq!(result.rejected_count, 1);
    assert!(result.rejected[0].reason.contains("parent directory"));
    assert!(result.fetched_size > 0);
}

#[tokio::test]
async fn scan_refuses_a_real_archive_with_an_extremely_deep_path() {
    let deep_name = "a/".repeat(32_000);
    let url = serve(build_zip(&[stored(&deep_name, ""), stored("ok.txt", "ok")]))
        .await
        .url;

    let result = scan(&url).await;
    assert_eq!(result.rejected_count, 1);
    assert_eq!(result.nodes.len(), 1);
}

#[tokio::test]
async fn selection_keeps_structure_relative_to_its_base() {
    let url = serve(build_zip(&[
        stored("data/2024/q3/a.csv", "aaa"),
        stored("data/2024/b.csv", "bb"),
        stored("data/2023/c.csv", "c"),
    ]))
    .await
    .url;
    let out = tempfile::tempdir().unwrap();

    let (report, failures) = download_selection(
        &url,
        out.path(),
        "data",
        &["data/2024/q3/a.csv", "data/2024/b.csv"],
    )
    .await;

    assert_eq!(failures, []);
    assert_eq!((report.completed, report.failed), (2, 0));
    assert_eq!(report.written_bytes, 5);
    assert!(report.network_bytes > 0);
    assert_eq!(
        std::fs::read(out.path().join("2024/q3/a.csv")).unwrap(),
        b"aaa"
    );
    assert_eq!(std::fs::read(out.path().join("2024/b.csv")).unwrap(), b"bb");
    assert_eq!(dir_entries(out.path()), ["2024"]);
}

#[tokio::test]
async fn one_missing_file_does_not_stop_the_others() {
    let url = serve(build_zip(&[stored("a.txt", "a"), stored("b.txt", "b")]))
        .await
        .url;
    let out = tempfile::tempdir().unwrap();

    let (report, failures) =
        download_selection(&url, out.path(), "", &["a.txt", "gone.txt", "b.txt"]).await;

    assert_eq!((report.completed, report.failed), (2, 1));
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "gone.txt");
    assert_eq!(dir_entries(out.path()), ["a.txt", "b.txt"]);
}

#[tokio::test]
async fn failed_download_keeps_the_existing_file_and_leaves_no_partial() {
    let big = vec![7u8; 3 * 1024 * 1024];
    let url = serve_with(
        build_zip(&[stored("big.bin", big)]),
        // Enough for the end of the file and the first chunk of the entry, but not the rest.
        Behavior {
            fail_after: Some((2, 503)),
            ..Default::default()
        },
    )
    .await
    .url;
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("big.bin"), b"good copy").unwrap();

    let (report, failures) = download_selection(&url, out.path(), "", &["big.bin"]).await;

    assert_eq!((report.completed, report.failed), (0, 1));
    assert_eq!(failures.len(), 1);
    assert_eq!(
        std::fs::read(out.path().join("big.bin")).unwrap(),
        b"good copy"
    );
    assert_eq!(dir_entries(out.path()), ["big.bin"]);
}

#[tokio::test]
async fn cancelling_stops_the_job_and_removes_partial_files() {
    let big = vec![7u8; 8 * 1024 * 1024];
    let url = serve_with(
        build_zip(&[stored("big.bin", big), stored("later.txt", "later")]),
        Behavior {
            get_delay: Some(Duration::from_millis(40)),
            ..Default::default()
        },
    )
    .await
    .url;
    let out = tempfile::tempdir().unwrap();
    let plan = plan_selection(
        out.path(),
        "",
        vec!["big.bin".to_string(), "later.txt".to_string()],
    );
    let cancel = CancellationToken::new();
    let batch_options = BatchOptions {
        concurrency: 1,
        cancel: cancel.clone(),
        root: Some(out.path().to_path_buf()),
        reader: fast_config(),
    };
    let bytes_seen = AtomicU32::new(0);

    let report = download_batch(&url, plan.files, batch_options, |event| {
        if let BatchEvent::FileBytes { .. } = event {
            bytes_seen.fetch_add(1, Ordering::SeqCst);
            cancel.cancel();
        }
    })
    .await;

    assert!(bytes_seen.load(Ordering::SeqCst) > 0);
    assert!(report.cancelled);
    assert_eq!((report.completed, report.failed), (0, 0));
    assert_eq!(dir_entries(out.path()), Vec::<String>::new());
}

#[tokio::test]
async fn zero_concurrency_does_not_hang() {
    let url = serve(build_zip(&[stored("a.txt", "a")])).await.url;
    let out = tempfile::tempdir().unwrap();
    let batch_options = BatchOptions {
        concurrency: 0,
        ..options(None)
    };
    let files = vec![BatchFile {
        zip_path: "a.txt".to_string(),
        dest: out.path().join("a.txt"),
    }];

    let report = tokio::time::timeout(
        Duration::from_secs(10),
        download_batch(&url, files, batch_options, |_| {}),
    )
    .await
    .expect("batch hung");
    assert_eq!(report.completed, 1);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_on_disk_cannot_redirect_a_write_outside_the_root() {
    let url = serve(build_zip(&[stored("link/evil.txt", "evil")]))
        .await
        .url;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let outside = temp.path().join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

    let (report, failures) = download_selection(&url, &root, "", &["link/evil.txt"]).await;

    assert_eq!((report.completed, report.failed), (0, 1));
    assert!(failures[0].1.contains("outside"), "{failures:?}");
    assert_eq!(dir_entries(&outside), Vec::<String>::new());
}

#[test]
fn plan_rejects_traversal_and_case_collisions() {
    let plan = plan_selection(
        Path::new("/out"),
        "",
        vec![
            "a/b/../../../evil".to_string(),
            "/abs".to_string(),
            "Report.txt".to_string(),
            "report.txt".to_string(),
        ],
    );
    assert_eq!(plan.files.len(), 1);
    assert_eq!(plan.files[0].dest, Path::new("/out/Report.txt"));
    let rejected: Vec<_> = plan.rejected.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(rejected, ["a/b/../../../evil", "/abs", "report.txt"]);
}

#[tokio::test]
async fn find_existing_reports_only_files_on_disk() {
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join("there.txt"), b"x").unwrap();
    let plan = plan_selection(
        out.path(),
        "",
        vec!["there.txt".to_string(), "missing.txt".to_string()],
    );
    assert_eq!(find_existing(&plan.files).await, ["there.txt"]);
}

#[tokio::test]
async fn deflated_files_are_decompressed_and_verified() {
    let contents = "repeated text ".repeat(50_000);
    let url = serve(build_zip(&[deflated("text.txt", contents.clone())]))
        .await
        .url;
    let out = tempfile::tempdir().unwrap();
    let (report, failures) = download_selection(&url, out.path(), "", &["text.txt"]).await;
    assert_eq!(failures, []);
    assert_eq!(report.completed, 1);
    assert_eq!(
        std::fs::read_to_string(out.path().join("text.txt")).unwrap(),
        contents
    );
}

async fn download_expecting_failure(entry: Entry) -> String {
    let name = entry.name.clone();
    let url = serve(build_zip(&[entry])).await.url;
    let out = tempfile::tempdir().unwrap();
    std::fs::write(out.path().join(&name), b"good copy").unwrap();
    let (report, failures) = download_selection(&url, out.path(), "", &[name.as_str()]).await;
    assert_eq!((report.completed, report.failed), (0, 1), "{failures:?}");
    assert_eq!(
        std::fs::read(out.path().join(&name)).unwrap(),
        b"good copy",
        "an existing file must survive a failed download"
    );
    assert_eq!(dir_entries(out.path()), [name]);
    failures.into_iter().next().unwrap().1
}

#[tokio::test]
async fn a_wrong_checksum_fails_and_keeps_the_existing_file() {
    let mut entry = deflated("data.bin", vec![1u8; 4096]);
    entry.wrong_crc = true;
    assert!(download_expecting_failure(entry).await.contains("checksum"));
}

#[tokio::test]
async fn a_zip_bomb_lying_about_its_size_stops_at_the_declared_size() {
    // Inflates to 5 MB while the directory promises 1 KB.
    let mut entry = deflated("bomb.bin", vec![0u8; 5 * 1024 * 1024]);
    entry.declared_size = Some(1024);
    let error = download_expecting_failure(entry).await;
    assert!(error.contains("expected 1024 bytes"), "{error}");
}

#[tokio::test]
async fn an_entry_smaller_than_declared_fails() {
    let mut entry = deflated("short.bin", vec![1u8; 1024]);
    entry.declared_size = Some(4096);
    let error = download_expecting_failure(entry).await;
    assert!(error.contains("expected 4096 bytes"), "{error}");
}

#[tokio::test]
async fn an_impossible_compression_ratio_is_refused_before_downloading() {
    let mut entry = deflated("bomb.bin", vec![0u8; 1024]);
    entry.declared_size = Some(100 * 1024 * 1024);
    assert!(download_expecting_failure(entry).await.contains("ratio"));
}

#[tokio::test]
async fn a_stored_entry_with_mismatched_sizes_is_refused() {
    let mut entry = stored("odd.bin", vec![0u8; 1024]);
    entry.declared_size = Some(2048);
    assert!(download_expecting_failure(entry).await.contains("sizes"));
}

#[tokio::test]
async fn a_server_failing_mid_batch_reports_each_file() {
    let url = serve_with(
        build_zip(&[stored("a.txt", "a"), stored("b.txt", "b")]),
        Behavior {
            fail_after: Some((0, 404)),
            ..Default::default()
        },
    )
    .await
    .url;
    let out = tempfile::tempdir().unwrap();
    let (report, failures) = download_selection(&url, out.path(), "", &["a.txt", "b.txt"]).await;
    assert_eq!(report.failed, 2);
    assert!(failures[0].1.contains("404"), "{failures:?}");
}

#[tokio::test]
async fn a_single_download_stays_within_its_byte_budget() {
    let mut entries: Vec<Entry> = (0..200)
        .map(|i| stored(&format!("small/{i}.txt"), "x"))
        .collect();
    entries.push(stored("big.bin", vec![0u8; 50 * 1024 * 1024]));
    let server = serve(build_zip(&entries)).await;
    let out = tempfile::tempdir().unwrap();
    let (report, _) = download_selection(&server.url, out.path(), "", &["small/7.txt"]).await;
    assert_eq!(report.completed, 1);
    // The end of the file, plus at most one fixed-size chunk for the entry.
    assert!(
        report.network_bytes < 1024 * 1024 + 128 * 1024,
        "used {} bytes",
        report.network_bytes
    );
}

#[tokio::test]
async fn files_share_one_connection_config_and_count_network_use_together() {
    let server = serve(build_zip(&[stored("a.txt", "a"), stored("b.txt", "b")])).await;
    let out = tempfile::tempdir().unwrap();
    let (report, _) = download_selection(&server.url, out.path(), "", &["a.txt", "b.txt"]).await;
    assert_eq!(report.completed, 2);
    assert_eq!(report.network_bytes as usize, server.body_bytes());
}

#[cfg(windows)]
#[tokio::test]
async fn downloads_are_marked_as_coming_from_the_internet() {
    let url = serve(build_zip(&[stored("a.txt", "a")])).await.url;
    let signed = format!("{url}?signature=SECRET");
    let out = tempfile::tempdir().unwrap();
    let (report, _) = download_selection(&signed, out.path(), "", &["a.txt"]).await;
    assert_eq!(report.completed, 1);
    let stream = out.path().join("a.txt:Zone.Identifier");
    let contents = std::fs::read_to_string(stream).unwrap();
    assert!(contents.contains("ZoneId=3"), "{contents}");
    assert!(!contents.contains("SECRET"), "{contents}");
}

#[tokio::test]
async fn every_supported_compression_method_extracts_correctly() {
    let archive = include_bytes!("data/methods.zip").to_vec();
    let url = serve(archive).await.url;
    let out = tempfile::tempdir().unwrap();
    let names = [
        "stored.txt",
        "deflate.txt",
        "bzip2.txt",
        "zstd.txt",
        "xz.txt",
    ];

    let (report, failures) = download_selection(&url, out.path(), "", &names).await;

    assert_eq!(failures, []);
    assert_eq!(report.completed, 5);
    let expected = "Every compression method should extract to exactly this text.\n".repeat(40);
    for name in names {
        assert_eq!(
            std::fs::read_to_string(out.path().join(name)).unwrap(),
            expected,
            "{name}"
        );
    }
}

#[tokio::test]
async fn lzma_is_listed_and_reports_a_clear_error() {
    let url = serve(include_bytes!("data/methods.zip").to_vec()).await.url;
    assert_eq!(scan(&url).await.nodes.len(), 6);

    let out = tempfile::tempdir().unwrap();
    let (report, failures) = download_selection(&url, out.path(), "", &["lzma.txt"]).await;
    assert_eq!((report.completed, report.failed), (0, 1));
    assert!(failures[0].1.contains("LZMA"), "{failures:?}");
    assert_eq!(dir_entries(out.path()), Vec::<String>::new());
}
