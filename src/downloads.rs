//! Streaming archive downloads. Finished archives are retained for installation.
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;
pub use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub received: u64,
    pub total: Option<u64>,
}

pub fn validate_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Unsupported archive URL")?;
    if url.scheme() != "https"
        || url.host_str() != Some("downloads.gtnewhorizons.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.path().starts_with("/Multi_mc_downloads/")
            || url.path().starts_with("/ServerPacks/"))
        || !url.path().ends_with(".zip")
        || url.path().contains('%')
        || url.path().contains('\\')
    {
        return Err("Unsupported archive URL".into());
    }
    Ok(url)
}

pub async fn download(
    url: &str,
    directory: &Path,
    cancel: CancellationToken,
    progress: impl Fn(DownloadProgress) + Send,
) -> Result<PathBuf, String> {
    validate_url(url)?;
    let client = archive_client()?;
    transfer(&client, url, directory, cancel, progress).await
}

fn archive_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("GTNH-Updater/0.1.0")
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("Too many archive redirects")
            } else if validate_url(attempt.url().as_str()).is_err() {
                attempt.error("Unsupported archive redirect")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

async fn transfer(
    client: &reqwest::Client,
    url: &str,
    directory: &Path,
    cancel: CancellationToken,
    progress: impl Fn(DownloadProgress) + Send,
) -> Result<PathBuf, String> {
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|e| e.to_string())?;
    // No server-controlled filename; TempPath closes its original handle immediately.
    let temporary = tempfile::Builder::new()
        .prefix("gtnh-")
        .suffix(".part")
        .tempfile_in(directory)
        .map_err(|e| e.to_string())?
        .into_temp_path();
    let destination = temporary.with_extension("zip");
    let operation = async {
        let mut response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err("Download cancelled".into()),
            response = client.get(url).send() => response.map_err(|e| e.to_string())?,
        };
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!("Archive HTTP {}", response.status()));
        }
        let total = response.content_length();
        let mut received = 0u64;
        progress(DownloadProgress { received, total });
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&temporary)
            .await
            .map_err(|e| e.to_string())?;
        loop {
            let chunk = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err("Download cancelled".into()),
                chunk = response.chunk() => chunk.map_err(|e| e.to_string())?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            // Finish pending filesystem I/O before allowing cancellation/cleanup.
            file.flush().await.map_err(|e| e.to_string())?;
            received += chunk.len() as u64;
            progress(DownloadProgress { received, total });
        }
        if received == 0 || total.is_some_and(|total| total != received) {
            return Err("Incomplete archive download".into());
        }
        file.flush().await.map_err(|e| e.to_string())?;
        file.sync_all().await.map_err(|e| e.to_string())?;
        // Close before rename/remove (required on Windows).
        drop(file);
        Ok::<(), String>(())
    };
    operation.await?;
    // Publication does not yield: cancellation cannot delete a published archive.
    if cancel.is_cancelled() {
        return Err("Download cancelled".into());
    }
    temporary
        .persist_noclobber(&destination)
        .map_err(|e| e.to_string())?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn only_official_archives_are_allowed() {
        for url in [
            "https://downloads.gtnewhorizons.com/Multi_mc_downloads/pack.zip",
            "https://downloads.gtnewhorizons.com/ServerPacks/betas/pack.zip",
        ] {
            assert!(validate_url(url).is_ok(), "{url}");
        }
        for url in [
            "http://downloads.gtnewhorizons.com/ServerPacks/a.zip",
            "https://evil.test/ServerPacks/a.zip",
            "https://downloads.gtnewhorizons.com.evil.test/ServerPacks/a.zip",
            "https://user@downloads.gtnewhorizons.com/ServerPacks/a.zip",
            "https://downloads.gtnewhorizons.com:444/ServerPacks/a.zip",
            "https://downloads.gtnewhorizons.com/other/a.zip",
            "https://downloads.gtnewhorizons.com/ServerPacks/a.exe",
            "https://downloads.gtnewhorizons.com/ServerPacks/%2e%2e%2fsecret.zip",
            "https://downloads.gtnewhorizons.com/ServerPacks/a.zip?target=other",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
    }

    async fn fixture(
        response: &'static [u8],
        stall: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket.write_all(response).await;
            if stall {
                std::future::pending::<()>().await;
            }
        });
        (format!("http://{address}/../../unsafe.zip"), task)
    }
    fn client() -> reqwest::Client {
        reqwest::Client::new()
    }
    fn files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|f| f.unwrap().path())
            .collect()
    }
    #[tokio::test]
    async fn writes_exact_bytes_reports_progress_and_keeps_existing_archives() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.zip");
        std::fs::write(&existing, b"old").unwrap();
        let (url, server) = fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nContent-Disposition: attachment; filename=../../escape.zip\r\n\r\nPACK", false).await;
        let progress = std::sync::Mutex::new(Vec::new());
        let path = transfer(&client(), &url, dir.path(), CancellationToken::new(), |p| {
            progress.lock().unwrap().push(p)
        })
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"PACK");
        assert_eq!(path.parent(), Some(dir.path()));
        assert_eq!(path.extension().unwrap(), "zip");
        assert_eq!(std::fs::read(existing).unwrap(), b"old");
        assert_eq!(files(dir.path()).len(), 2);
        let updates = progress.lock().unwrap();
        let last = updates.last().unwrap();
        assert_eq!(last.received, 4);
        assert_eq!(last.total, Some(4));
    }
    #[tokio::test]
    async fn supports_unknown_size() {
        let dir = tempfile::tempdir().unwrap();
        let (url, server) =
            fixture(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nPACK", false).await;
        let updates = std::sync::Mutex::new(Vec::new());
        let path = transfer(&client(), &url, dir.path(), CancellationToken::new(), |p| {
            updates.lock().unwrap().push(p)
        })
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"PACK");
        assert!(updates.lock().unwrap().iter().all(|p| p.total.is_none()));
    }
    #[tokio::test]
    async fn errors_remove_partial_files() {
        for response in [
            &b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort"[..],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (url, server) = fixture(response, false).await;
            assert!(transfer(
                &client(),
                &url,
                dir.path(),
                CancellationToken::new(),
                |_| {}
            )
            .await
            .is_err());
            server.await.unwrap();
            assert!(files(dir.path()).is_empty());
        }
    }
    #[tokio::test]
    async fn cancellation_interrupts_headers_and_body_and_cleans_up() {
        for response in [
            &b""[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort"[..],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let (url, server) = fixture(response, true).await;
            let cancel = CancellationToken::new();
            let token = cancel.clone();
            let timer = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                token.cancel();
            });
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                transfer(&client(), &url, dir.path(), cancel, |_| {}),
            )
            .await
            .unwrap();
            assert!(result.is_err());
            assert!(files(dir.path()).is_empty());
            timer.await.unwrap();
            server.abort();
        }
    }
    #[tokio::test]
    async fn redirects_cannot_escape_the_archive_policy() {
        let (url, server) = fixture(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/escape.zip\r\nContent-Length: 0\r\n\r\n", false).await;
        let error = archive_client().unwrap().get(url).send().await.unwrap_err();
        assert!(error.is_redirect(), "{error:?}");
        server.await.unwrap();
    }
    #[tokio::test]
    async fn concurrent_downloads_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let (a, sa) = fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nA", false).await;
        let (b, sb) = fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nB", false).await;
        let client = client();
        let (a, b) = tokio::join!(
            transfer(&client, &a, dir.path(), CancellationToken::new(), |_| {}),
            transfer(&client, &b, dir.path(), CancellationToken::new(), |_| {})
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_ne!(a, b);
        assert_eq!(std::fs::read(a).unwrap(), b"A");
        assert_eq!(std::fs::read(b).unwrap(), b"B");
        assert_eq!(files(dir.path()).len(), 2);
        sa.await.unwrap();
        sb.await.unwrap();
    }
}
