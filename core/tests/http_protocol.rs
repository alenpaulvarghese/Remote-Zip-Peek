//! How the reader copes with servers that are unusual, broken or hostile.

mod support;

use lazy_zip_core::error::Error;
use lazy_zip_core::http_reader::RemoteHttpReader;
use lazy_zip_core::zip_explorer::{ScanResult, ZipExplorer};
use std::io::SeekFrom;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use support::*;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

async fn scan(url: &str) -> Result<ScanResult, Error> {
    let reader = RemoteHttpReader::open(url, fast_config()).await?;
    ZipExplorer::new(reader).list_files().await
}

fn small_zip() -> Vec<u8> {
    build_zip(&[stored("a.txt", "hello"), stored("dir/b.txt", "world")])
}

/// An archive whose directory doesn't fit in the first fetch from the end of the file.
fn large_directory_zip() -> Vec<u8> {
    let entries: Vec<Entry> = (0..5000)
        .map(|i| stored(&format!("folder/entry-with-a-long-name-{i:05}.txt"), "x"))
        .collect();
    build_zip(&entries)
}

#[tokio::test]
async fn sends_a_user_agent_and_never_needs_head() {
    let server = serve_with(
        small_zip(),
        Behavior {
            reject_head: true,
            ..Default::default()
        },
    )
    .await;

    let result = scan(&server.url).await.unwrap();

    assert_eq!(result.nodes.len(), 3);
    let requests = server.requests();
    assert!(requests.iter().all(|r| r.method == "GET"), "{requests:?}");
    assert!(requests[0]
        .user_agent
        .as_deref()
        .is_some_and(|agent| agent.starts_with("lazy-zip/")));
}

#[tokio::test]
async fn small_archive_is_listed_with_a_single_request() {
    let server = serve(small_zip()).await;
    scan(&server.url).await.unwrap();
    assert_eq!(server.gets(), 1, "{:?}", server.requests());
}

#[tokio::test]
async fn a_maximum_length_comment_costs_one_request() {
    let archive = build_zip_with_comment(&[stored("a.txt", "x")], &vec![b'c'; 65535]);
    let server = serve(archive).await;
    let result = scan(&server.url).await.unwrap();
    assert_eq!(result.nodes.len(), 1);
    assert_eq!(server.gets(), 1);
}

#[tokio::test]
async fn server_ignoring_range_stops_after_reading_no_data() {
    let archive = vec![7u8; 50 * 1024 * 1024];
    let server = serve_with(
        archive,
        Behavior {
            ignore_range: true,
            ..Default::default()
        },
    )
    .await;
    let config = fast_config();
    let network = config.network_bytes.clone();

    let error = match RemoteHttpReader::open(&server.url, config).await {
        Err(error) => error,
        Ok(_) => panic!("expected an error"),
    };

    assert!(matches!(error, Error::RangeNotSupported), "{error}");
    assert_eq!(network.load(Ordering::Relaxed), 0);
    assert!(server.gets() <= 2, "{}", server.gets());
}

#[tokio::test]
async fn servers_without_suffix_ranges_still_work() {
    let refusing = [400, 416, 501].map(|status| Behavior {
        suffix_range_status: Some(status),
        ..Default::default()
    });
    let ignoring = Behavior {
        no_suffix_ranges: true,
        ..Default::default()
    };
    for behavior in refusing.into_iter().chain([ignoring]) {
        let server = serve_with(small_zip(), behavior).await;
        assert_eq!(scan(&server.url).await.unwrap().nodes.len(), 3);
    }
}

#[tokio::test]
async fn permanent_errors_on_the_suffix_range_are_not_treated_as_missing_support() {
    for status in [401, 403, 404, 410] {
        let server = serve_with(
            small_zip(),
            Behavior {
                suffix_range_status: Some(status),
                ..Default::default()
            },
        )
        .await;
        assert!(
            matches!(scan(&server.url).await, Err(Error::HttpStatus { status: s }) if s == status),
            "{status}"
        );
        assert_eq!(server.gets(), 1);
    }
}

#[tokio::test]
async fn a_non_zip_is_reported_clearly() {
    let server = serve(b"<html>this is a web page</html>".to_vec()).await;
    assert!(matches!(scan(&server.url).await, Err(Error::NotAZip)));
}

#[tokio::test]
async fn an_empty_file_is_reported_clearly() {
    let server = serve(Vec::new()).await;
    assert!(matches!(scan(&server.url).await, Err(Error::NotAZip)));
}

#[tokio::test]
async fn http_errors_keep_their_status_and_are_not_retried_when_permanent() {
    let server = serve_with(
        small_zip(),
        Behavior {
            fail_first: Some((100, 404, None)),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(
        scan(&server.url).await,
        Err(Error::HttpStatus { status: 404 })
    ));
    assert_eq!(server.gets(), 1);
}

#[tokio::test]
async fn transient_errors_are_retried() {
    for status in [429, 500, 502, 503, 504] {
        let server = serve_with(
            small_zip(),
            Behavior {
                fail_first: Some((2, status, None)),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(scan(&server.url).await.unwrap().nodes.len(), 3, "{status}");
        assert_eq!(server.gets(), 3, "{status}");
    }
}

#[tokio::test]
async fn retries_give_up_with_the_last_error() {
    let server = serve_with(
        small_zip(),
        Behavior {
            fail_first: Some((100, 503, None)),
            ..Default::default()
        },
    )
    .await;
    assert!(matches!(
        scan(&server.url).await,
        Err(Error::HttpStatus { status: 503 })
    ));
    assert_eq!(server.gets(), 3);
}

#[tokio::test]
async fn retry_after_is_honored() {
    let server = serve_with(
        small_zip(),
        Behavior {
            fail_first: Some((1, 429, Some(1))),
            ..Default::default()
        },
    )
    .await;
    let started = Instant::now();
    scan(&server.url).await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[tokio::test]
async fn a_stalled_server_times_out() {
    let server = serve_with(
        small_zip(),
        Behavior {
            stall_after: Some(0),
            ..Default::default()
        },
    )
    .await;
    let mut config = fast_config();
    config.retry.max_attempts = 1;
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        RemoteHttpReader::open(&server.url, config),
    )
    .await
    .expect("the reader should give up on its own");
    assert!(matches!(result, Err(Error::Http(_))));
}

#[tokio::test]
async fn network_errors_do_not_expose_signed_urls() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let url = format!("http://{address}/a.zip?signature=SECRET");
    let mut config = fast_config();
    config.retry.max_attempts = 1;
    let error = match RemoteHttpReader::open(&url, config).await {
        Err(error) => error,
        Ok(_) => panic!("expected an error"),
    };
    assert!(!error.to_string().contains("SECRET"), "{error}");
    // The cause depends on the platform: Linux and macOS refuse the connection, Windows times out.
    assert!(
        !error.to_string().contains("error sending request"),
        "should name the cause: {error}"
    );
}

/// Reads part of the file that isn't covered by the tail fetched at open.
async fn read_start_of_archive(behavior: Behavior) -> Result<Vec<u8>, Error> {
    let archive = large_directory_zip();
    let expected_len = archive.len();
    assert!(expected_len as u64 > lazy_zip_core::archive_limits::TAIL_BYTES);
    let server = serve_with(archive, behavior).await;
    let mut reader = RemoteHttpReader::open(&server.url, fast_config()).await?;
    let mut data = vec![0u8; 64];
    reader.read_exact(&mut data).await?;
    Ok(data)
}

#[tokio::test]
async fn reads_correct_data_from_a_good_server() {
    let data = read_start_of_archive(Behavior::default()).await.unwrap();
    assert_eq!(&data[..4], b"PK\x03\x04");
}

#[tokio::test]
async fn a_response_for_the_wrong_range_is_rejected() {
    // A shift would corrupt data silently if the Content-Range weren't checked. The tail fetched at
    // open tolerates it because a suffix is clamped at the end of the file.
    let error = read_start_of_archive(Behavior {
        shift_range: 10,
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(error, Error::InvalidResponse(_)), "{error}");
}

#[tokio::test]
async fn compressed_partial_responses_are_rejected() {
    let error = read_start_of_archive(Behavior {
        content_encoding: Some("gzip"),
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(error, Error::InvalidResponse(_)), "{error}");
}

#[tokio::test]
async fn an_empty_body_is_an_error_not_an_endless_loop() {
    let server = serve_with(
        small_zip(),
        Behavior {
            empty_body: true,
            ..Default::default()
        },
    )
    .await;
    let result = tokio::time::timeout(Duration::from_secs(10), scan(&server.url))
        .await
        .expect("must not loop forever");
    assert!(result.is_err());
    assert!(server.gets() <= 3, "{}", server.gets());
}

#[tokio::test]
async fn a_truncated_body_is_an_error_after_retrying() {
    let error = read_start_of_archive(Behavior {
        truncate_after: Some(10),
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(error, Error::TruncatedResponse), "{error}");
}

#[tokio::test]
async fn a_body_longer_than_promised_is_not_buffered() {
    let error = read_start_of_archive(Behavior {
        extra_body_bytes: 10 * 1024 * 1024,
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(error, Error::InvalidResponse(_)), "{error}");
}

#[tokio::test]
async fn a_file_changing_mid_read_is_detected_by_etag() {
    let error = read_start_of_archive(Behavior {
        etag: Some("\"v1\""),
        change_etag_after: Some(1),
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(error, Error::RemoteChanged), "{error}");
}

#[tokio::test]
async fn a_file_changing_size_mid_read_is_detected() {
    let error = read_start_of_archive(Behavior {
        change_length_after: Some(1),
        ..Default::default()
    })
    .await
    .unwrap_err();
    assert!(matches!(error, Error::RemoteChanged), "{error}");
}

#[tokio::test]
async fn seeking_matches_a_local_file() {
    let archive = large_directory_zip();
    let server = serve(archive.clone()).await;
    let mut reader = RemoteHttpReader::open(&server.url, fast_config())
        .await
        .unwrap();

    let mut buffer = [0u8; 16];
    reader.seek(SeekFrom::Start(100)).await.unwrap();
    reader.read_exact(&mut buffer).await.unwrap();
    assert_eq!(buffer, archive[100..116]);

    reader.seek(SeekFrom::Current(-8)).await.unwrap();
    reader.read_exact(&mut buffer).await.unwrap();
    assert_eq!(buffer, archive[108..124]);

    let position = reader.seek(SeekFrom::End(-16)).await.unwrap();
    assert_eq!(position as usize, archive.len() - 16);
    reader.read_exact(&mut buffer).await.unwrap();
    assert_eq!(buffer, archive[archive.len() - 16..]);

    assert_eq!(reader.read(&mut buffer).await.unwrap(), 0);
    assert!(reader.seek(SeekFrom::Start(0)).await.is_ok());
}

#[tokio::test]
async fn seeking_before_the_start_is_an_error() {
    let server = serve(small_zip()).await;
    let mut reader = RemoteHttpReader::open(&server.url, fast_config())
        .await
        .unwrap();
    assert!(reader.seek(SeekFrom::End(-1_000_000)).await.is_err());
    reader.seek(SeekFrom::Start(0)).await.unwrap();
    assert!(reader.seek(SeekFrom::Current(-1)).await.is_err());
}

#[tokio::test]
async fn reads_up_to_the_tail_without_fetching_it_again() {
    let archive = large_directory_zip();
    let server = serve(archive.clone()).await;
    let mut reader = RemoteHttpReader::open(&server.url, fast_config())
        .await
        .unwrap();
    let mut everything = Vec::new();
    reader.read_to_end(&mut everything).await.unwrap();
    assert_eq!(everything, archive);
    let tail_refetched = server
        .requests()
        .iter()
        .skip(1)
        .filter_map(|r| r.range.clone())
        .filter_map(|r| r.split('-').nth(1)?.parse::<usize>().ok())
        .any(|end| end + 1 == archive.len());
    assert!(!tail_refetched, "{:?}", server.requests());
}

// Regression budgets for the product's core promise. They count bytes, so they aren't flaky.

#[tokio::test]
async fn listing_stays_cheap() {
    let archive = large_directory_zip();
    let directory_bytes = archive.len() / 2 + 1;
    let padded = {
        let mut entries: Vec<Entry> = (0..5000)
            .map(|i| stored(&format!("folder/entry-with-a-long-name-{i:05}.txt"), "x"))
            .collect();
        entries.push(stored("big.bin", vec![0u8; 100 * 1024 * 1024]));
        build_zip(&entries)
    };
    let server = serve(padded.clone()).await;
    let config = fast_config();
    let network = config.network_bytes.clone();
    let reader = RemoteHttpReader::open(&server.url, config).await.unwrap();
    let result = ZipExplorer::new(reader).list_files().await.unwrap();

    assert_eq!(result.nodes.len(), 5002);
    let used = network.load(Ordering::Relaxed);
    assert!(used < 2 * 1024 * 1024, "listing used {used} bytes");
    assert!(server.gets() <= 4, "{}", server.gets());
    assert_eq!(result.fetched_size, used);
    assert!(directory_bytes > 0 && (used as usize) < padded.len() / 50);
}

#[tokio::test]
async fn directory_parsing_cannot_fetch_without_limit() {
    // The directory lies about where it starts, so the parser reads from the start of a huge file.
    // A record that can't be parsed must fail promptly rather than reading everything.
    let mut archive = vec![b'x'; 8 * 1024 * 1024];
    let eocd_offset = archive.len();
    archive.extend_from_slice(b"PK\x05\x06");
    archive.extend_from_slice(&[0; 4]);
    archive.extend_from_slice(&2u16.to_le_bytes());
    archive.extend_from_slice(&2u16.to_le_bytes());
    archive.extend_from_slice(&(eocd_offset as u32).to_le_bytes());
    archive.extend_from_slice(&0u32.to_le_bytes());
    archive.extend_from_slice(&[0; 2]);
    let server = serve(archive).await;
    assert!(scan(&server.url).await.is_err());
    assert!(
        server.body_bytes() < 2 * 1024 * 1024,
        "{}",
        server.body_bytes()
    );
}
