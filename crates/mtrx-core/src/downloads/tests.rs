use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpListener,
};

use matrix_sdk::{media::store::IgnoreMediaRetentionPolicy, ruma::owned_room_id};
use matrix_sdk_crypto::AttachmentEncryptor;
use tempfile::TempDir;

use super::*;

fn media(filename: &str, size: Option<u64>) -> Media {
    Media {
        kind: MediaKind::File,
        filename: filename.into(),
        size,
        mimetype: None,
        width: None,
        height: None,
        source: MediaRef::new(&MediaSource::Plain("mxc://example.org/abc".into()), None),
    }
}

fn download(id: DownloadId, state: DownloadState) -> Download {
    let mut download = Download::new(
        id,
        &media("report.pdf", Some(10_000_000)),
        owned_room_id!("!room:example.org"),
        "Ice sheets".into(),
        None,
        SaveTo::Downloads,
    );
    download.state = state;
    download
}

fn bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn sizes_and_status_lines() {
    assert_eq!(human_size(512), "512 B");
    assert_eq!(human_size(2_400_000), "2.4 MB");
    assert_eq!(human_size(120_000), "120 kB");
    assert_eq!(human_size(999_960), "1.0 MB");

    let running = |received, total| {
        download(
            1,
            DownloadState::Downloading {
                received,
                total: Some(total),
            },
        )
    };
    assert_eq!(running(0, 10_000_000).status(), "Starting…");
    assert_eq!(running(0, 10_000_000).fraction(), Some(0.0));
    assert_eq!(running(3_200_000, 10_000_000).status(), "3.2 of 10.0 MB");
    assert_eq!(running(2_500_000, 10_000_000).fraction(), Some(0.25));
    let unknown = download(
        1,
        DownloadState::Downloading {
            received: 1_500,
            total: None,
        },
    );
    assert_eq!(unknown.status(), "1.5 kB");
    assert_eq!(unknown.fraction(), None);

    let done = download(
        1,
        DownloadState::Done {
            path: "/tmp/report.pdf".into(),
        },
    );
    assert_eq!(done.status(), "10.0 MB · Ice sheets");
    assert_eq!(done.fraction(), None);
    let failed = download(1, DownloadState::Failed(DAMAGED.into()));
    assert_eq!(failed.status(), "Failed: The file is damaged");
    assert_eq!(download(1, DownloadState::Cancelled).status(), "Cancelled");
}

#[test]
fn senders_names_are_made_safe() {
    assert_eq!(safe_name("report.pdf", None), "report.pdf");
    assert_eq!(safe_name("../../etc/passwd", None), "passwd");
    assert_eq!(safe_name("C:\\Users\\x\\notes.txt", None), "notes.txt");
    assert_eq!(safe_name("..hidden.txt", None), "hidden.txt");
    assert_eq!(safe_name("a\u{7}b?.pdf", None), "a_b_.pdf");
    assert_eq!(safe_name("report. ", None), "report");
    assert_eq!(safe_name("", None), "file");
    assert_eq!(safe_name("..", Some("application/pdf")), "file.pdf");
    assert_eq!(safe_name("CON.txt", None), "_CON.txt");
    assert_eq!(safe_name("com1", None), "_com1");
    assert_eq!(safe_name("console.txt", None), "console.txt");
    assert_eq!(safe_name("photo", Some("image/png")), "photo.png");
    assert_eq!(safe_name("notes.txt", Some("application/pdf")), "notes.txt");
    let long = safe_name(&format!("{}.pdf", "é".repeat(150)), None);
    assert!(long.len() <= 200, "{}", long.len());
    assert!(long.ends_with("é.pdf"));
}

#[test]
fn names_in_a_folder_are_unique() {
    let folder = TempDir::new().unwrap();
    let path = |name: &str| folder.path().join(name);
    assert_eq!(
        unique_path(folder.path(), "report.pdf", taken),
        path("report.pdf")
    );
    fs::write(path("report.pdf"), "").unwrap();
    // Being downloaded counts as taken.
    fs::write(path("report (1).pdf.part"), "").unwrap();
    assert_eq!(
        unique_path(folder.path(), "report.pdf", taken),
        path("report (2).pdf")
    );
    fs::write(path("README"), "").unwrap();
    assert_eq!(
        unique_path(folder.path(), "README", taken),
        path("README (1)")
    );
}

#[test]
fn the_list_keeps_running_downloads_and_the_newest_others() {
    let mut list: Vec<Download> = (0..30)
        .rev()
        .map(|id| {
            let state = if id % 10 == 0 {
                DownloadState::Downloading {
                    received: 0,
                    total: None,
                }
            } else {
                DownloadState::Cancelled
            };
            download(id, state)
        })
        .collect();
    trim(&mut list);
    let ids: Vec<DownloadId> = list.iter().map(|d| d.id).collect();
    // 29 down to 8 are the 20 newest that don't run (20 and 10 run); 0 runs.
    let mut want: Vec<DownloadId> = (8..30).rev().collect();
    want.push(0);
    assert_eq!(ids, want);
}

#[test]
fn reopening_fails_interrupted_downloads_and_missing_files() {
    let folder = TempDir::new().unwrap();
    let places = Places {
        downloads: Some(folder.path().into()),
        cache: folder.path().join("cache"),
    };
    let kept = folder.path().join("kept.pdf");
    fs::write(&kept, "").unwrap();
    let mut running = download(
        3,
        DownloadState::Downloading {
            received: 5,
            total: None,
        },
    );
    let target = folder.path().join("report.pdf");
    fs::write(part_path(&target), "half").unwrap();
    fs::create_dir_all(places.ciphertext(3).parent().unwrap()).unwrap();
    fs::write(places.ciphertext(3), "half").unwrap();
    running.target = Some(target.clone());
    let list = vec![
        running,
        download(2, DownloadState::Done { path: kept.clone() }),
        download(
            1,
            DownloadState::Done {
                path: folder.path().join("gone.pdf"),
            },
        ),
    ];

    let (list, changed) = reopened(list, &places);
    assert!(changed);
    assert_eq!(list[0].state, DownloadState::Failed(QUIT.into()));
    assert_eq!(list[0].target, None);
    assert!(!part_path(&target).exists());
    assert!(!places.ciphertext(3).exists());
    assert_eq!(list[1].state, DownloadState::Done { path: kept });
    assert_eq!(list[2].state, DownloadState::Failed(GONE.into()));

    let (_, changed) = reopened(list, &places);
    assert!(!changed);
}

#[test]
fn progress_is_reported_ten_times_a_second_at_most() {
    let start = Instant::now();
    let mut throttle = Throttle::new(STEP, start);
    let at = |ms| start + Duration::from_millis(ms);
    let due: Vec<bool> = [10, 99, 100, 150, 199, 200, 350]
        .into_iter()
        .map(|ms| throttle.due(at(ms)))
        .collect();
    assert_eq!(due, [false, false, true, false, false, true, true]);
}

#[test]
fn encrypted_files_are_decrypted_and_checked() {
    let folder = TempDir::new().unwrap();
    let plain = bytes(700_000);
    let mut source = plain.as_slice();
    let mut encryptor = AttachmentEncryptor::new(&mut source);
    let mut ciphertext = Vec::new();
    encryptor.read_to_end(&mut ciphertext).unwrap();
    let key = serde_json::to_value(encryptor.finish()).unwrap();
    let key = || serde_json::from_value::<MediaEncryptionInfo>(key.clone()).unwrap();
    let (from, to) = (folder.path().join("in"), folder.path().join("out"));

    fs::write(&from, &ciphertext).unwrap();
    decrypt_file(&from, &to, key(), || true).unwrap();
    assert_eq!(fs::read(&to).unwrap(), plain);

    assert_eq!(
        decrypt_file(&from, &to, key(), || false),
        Err("Cancelled".into())
    );

    ciphertext[1000] ^= 1;
    fs::write(&from, &ciphertext).unwrap();
    assert_eq!(
        decrypt_file(&from, &to, key(), || true),
        Err(DAMAGED.into())
    );
}

/// A homeserver at a local port: `/versions` says Matrix 1.1 (so files
/// come from the unauthenticated endpoint), and any other request gets
/// `body`, or only its first `stall_after` bytes, then nothing.
fn serve(body: Vec<u8>, stall_after: Option<usize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { return };
            let body = body.clone();
            std::thread::spawn(move || {
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let versions = String::from_utf8_lossy(&request).contains("/versions");
                let body = if versions {
                    br#"{"versions":["v1.1"]}"#.to_vec()
                } else {
                    body
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes());
                match stall_after {
                    Some(sent) if !versions => {
                        let _ = socket.write_all(&body[..sent]);
                        let _ = socket.flush();
                        std::thread::sleep(Duration::from_secs(120));
                    }
                    _ => {
                        let _ = socket.write_all(&body);
                    }
                }
            });
        }
    });
    url
}

/// Downloads of a client of `homeserver`, with its folders.
async fn downloads(homeserver: &str) -> (Downloads, Client, TempDir) {
    let folder = TempDir::new().unwrap();
    let client = Client::builder()
        .homeserver_url(homeserver)
        .build()
        .await
        .unwrap();
    let places = Places {
        downloads: Some(folder.path().join("Downloads")),
        cache: folder.path().join("cache"),
    };
    let downloads = Downloads::load(client.clone(), places).await;
    (downloads, client, folder)
}

/// Nothing listens on the discard port.
const NO_SERVER: &str = "http://127.0.0.1:9";

fn start(downloads: &Downloads, media: &Media, to: SaveTo) -> DownloadId {
    downloads.start(
        media,
        owned_room_id!("!room:example.org"),
        "Ice sheets".into(),
        None,
        to,
    )
}

fn state(downloads: &Downloads, id: DownloadId) -> DownloadState {
    let list = downloads.shared.list.borrow();
    list.iter().find(|d| d.id == id).unwrap().state.clone()
}

async fn eventually(what: &str, done: impl Fn() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out: {what}");
}

async fn store_file(client: &Client, media: &Media, content: &[u8]) {
    let request = MediaRequestParameters {
        source: media.source.file.clone(),
        format: MediaFormat::File,
    };
    (client.media_store().lock().await.unwrap())
        .add_media_content(&request, content.to_vec(), IgnoreMediaRetentionPolicy::Yes)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_streams_to_its_place() {
    let body = bytes(2_000_000);
    let (downloads, _, folder) = downloads(&serve(body.clone(), None)).await;
    let target = folder.path().join("saved.bin");
    let file = media("data.bin", None);
    let id = start(&downloads, &file, SaveTo::File(target.clone()));
    assert!(matches!(
        state(&downloads, id),
        DownloadState::Downloading { received: 0, .. }
    ));
    // The same file to the same place is the same download.
    assert_eq!(start(&downloads, &file, SaveTo::File(target.clone())), id);

    assert_eq!(downloads.finished(id).await.unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), body);
    assert!(!part_path(&target).exists());
    assert_eq!(state(&downloads, id), DownloadState::Done { path: target });
}

#[tokio::test(flavor = "multi_thread")]
async fn downloads_get_unique_names() {
    let body = bytes(1000);
    let (downloads, _, folder) = downloads(&serve(body.clone(), Some(10))).await;
    let file = media("report.pdf", None);
    // Running at once, so neither file is there yet.
    let one = start(&downloads, &file, SaveTo::Downloads);
    let other = media("report.pdf", None);
    let two = downloads.start(
        &other,
        owned_room_id!("!other:example.org"),
        "Other".into(),
        None,
        SaveTo::File(folder.path().join("x")),
    );
    assert_ne!(one, two);
    let three = {
        let mut file = media("report.pdf", None);
        file.source = MediaRef::new(&MediaSource::Plain("mxc://example.org/def".into()), None);
        start(&downloads, &file, SaveTo::Downloads)
    };
    let list = downloads.shared.list.borrow().clone();
    let target = |id| list.iter().find(|d| d.id == id).unwrap().target.clone();
    let folder = folder.path().join("Downloads");
    assert_eq!(target(one), Some(folder.join("report.pdf")));
    assert_eq!(target(three), Some(folder.join("report (1).pdf")));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_deletes_the_partial_file() {
    let (downloads, _, folder) = downloads(&serve(bytes(1_000_000), Some(1000))).await;
    let target = folder.path().join("Downloads").join("data.bin");
    let id = start(&downloads, &media("data.bin", None), SaveTo::Downloads);
    let part = part_path(&target);
    eventually("the partial file", || part.exists()).await;

    downloads.cancel(id);
    assert_eq!(state(&downloads, id), DownloadState::Cancelled);
    eventually("the partial file deleted", || !part.exists()).await;
    assert!(!target.exists());
    assert_eq!(
        downloads.finished(id).await.unwrap_err().to_string(),
        "Cancelled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_list_is_kept_and_interrupted_downloads_fail() {
    let (downloads, client, folder) = downloads(&serve(bytes(1_000_000), Some(1000))).await;
    let file = media("data.bin", None);
    let running = start(&downloads, &file, SaveTo::Downloads);
    let part = part_path(&folder.path().join("Downloads").join("data.bin"));
    eventually("the partial file", || part.exists()).await;
    for _ in 0..200 {
        let stored = client.state_store().get_custom_value(STORE_KEY).await;
        if stored.unwrap().is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // As if the app quit.
    let places = downloads.shared.places.clone();
    drop(downloads);
    eventually("the partial file deleted", || !part.exists()).await;

    let reopened = Downloads::load(client.clone(), places).await;
    let list = reopened.shared.list.borrow().clone();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, running);
    assert_eq!(list[0].file_name, "data.bin");
    assert_eq!(list[0].state, DownloadState::Failed(QUIT.into()));
    // New downloads get new IDs.
    let mut other = media("other.bin", None);
    other.source = MediaRef::new(&MediaSource::Plain("mxc://example.org/def".into()), None);
    assert!(start(&reopened, &other, SaveTo::Downloads) > running);
}

#[tokio::test(flavor = "multi_thread")]
async fn files_the_media_store_has_are_written_from_it() {
    let (downloads, client, folder) = downloads(NO_SERVER).await;
    let file = media("notes.txt", Some(5));
    let target = folder.path().join("notes.txt");

    let failed = start(&downloads, &file, SaveTo::File(target.clone()));
    let error = downloads.finished(failed).await.unwrap_err();
    assert_eq!(error.to_string(), UNREACHABLE);
    assert!(!target.exists());

    // A viewer loaded it meanwhile: a retry needs no server.
    store_file(&client, &file, b"hello").await;
    downloads.retry(failed);
    assert_eq!(downloads.finished(failed).await.unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), b"hello");
}

#[tokio::test(flavor = "multi_thread")]
async fn files_in_the_cache_are_done_at_once() {
    let (downloads, client, folder) = downloads(NO_SERVER).await;
    let file = media("Ice Sheet.png", None);
    store_file(&client, &file, b"png").await;
    let id = start(&downloads, &file, SaveTo::Cache);
    let path = downloads.finished(id).await.unwrap();
    assert_eq!(
        path,
        folder
            .path()
            .join("cache/media/example.org/abc/Ice Sheet.png")
    );
    // Again: that download.
    assert_eq!(start(&downloads, &file, SaveTo::Cache), id);

    // Removed from the list, the file stays and counts as done.
    downloads.remove(id);
    assert!(downloads.shared.list.borrow().is_empty());
    let again = start(&downloads, &file, SaveTo::Cache);
    assert_ne!(again, id);
    assert_eq!(state(&downloads, again), DownloadState::Done { path });

    downloads.clear();
    assert!(downloads.shared.list.borrow().is_empty());
}
