use crate::models::DownloadedPdf;
use anyhow::{Context, Result};
use futures::{stream, StreamExt};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tracing::info;

#[derive(Debug, Clone)]
pub struct Downloader {
    max_size_bytes: u64,
}

impl Downloader {
    pub fn new() -> Result<Self> {
        Ok(Self {
            max_size_bytes: 100 * 1024 * 1024,
        })
    }

    pub fn with_max_size(mut self, max_size_mb: u64) -> Self {
        self.max_size_bytes = max_size_mb * 1024 * 1024;
        self
    }

    pub async fn download_pdf(&self, url: &str, dest_dir: &Path) -> Result<DownloadedPdf> {
        let response =
            crate::public_fetch::get_public(url, std::time::Duration::from_secs(120)).await?;
        self.save_response(url, dest_dir, response).await
    }

    async fn save_response(
        &self,
        url: &str,
        dest_dir: &Path,
        response: reqwest::Response,
    ) -> Result<DownloadedPdf> {
        response
            .error_for_status_ref()
            .context(format!("PDF server returned an error for: {}", url))?;

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if content_type != "application/pdf"
            && !content_type.contains("pdf")
            && !url.to_lowercase().ends_with(".pdf")
        {
            anyhow::bail!(
                "URL does not point to a PDF: {} (content-type: {})",
                url,
                content_type
            );
        }

        let content_length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);

        if content_length > self.max_size_bytes {
            anyhow::bail!(
                "PDF too large: {} bytes (max: {} bytes)",
                content_length,
                self.max_size_bytes
            );
        }

        tokio::fs::create_dir_all(dest_dir).await?;
        let temporary =
            TemporaryDownload(dest_dir.join(format!(".ayla-{}.part", uuid::Uuid::new_v4())));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary.0)
            .await?;
        let mut chunks = response.bytes_stream();
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut magic = Vec::with_capacity(4);
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.context("Failed to read PDF body")?;
            size = size
                .checked_add(chunk.len() as u64)
                .context("PDF size overflow")?;
            if size > self.max_size_bytes {
                anyhow::bail!("PDF exceeds max size limit during download");
            }
            if magic.len() < 4 {
                magic.extend_from_slice(&chunk[..chunk.len().min(4 - magic.len())]);
                if magic.len() == 4 && magic.as_slice() != b"%PDF" {
                    anyhow::bail!("Downloaded content is not a PDF");
                }
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        if magic.as_slice() != b"%PDF" {
            anyhow::bail!("Downloaded PDF is empty or truncated");
        }
        file.sync_all().await?;
        drop(file);
        let hash = format!("{:x}", hasher.finalize());
        let file_path = dest_dir.join(sanitize_filename(url, &hash));
        tokio::fs::rename(&temporary.0, &file_path).await?;
        info!("Downloaded PDF: {} bytes", size);
        Ok(DownloadedPdf {
            original_url: url.to_string(),
            file_path,
            sha256: hash,
            file_size_bytes: size,
        })
    }

    pub async fn download_parallel(
        &self,
        urls: &[String],
        dest_dir: &Path,
    ) -> Vec<Result<DownloadedPdf>> {
        // Owned futures are dropped on cancellation; no detached download tasks survive.
        stream::iter(
            urls.iter()
                .take(100)
                .map(|url| self.download_pdf(url, dest_dir)),
        )
        .buffer_unordered(4)
        .collect()
        .await
    }
}

struct TemporaryDownload(PathBuf);
impl Drop for TemporaryDownload {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn sanitize_filename(url: &str, hash: &str) -> String {
    let base = url
        .rsplit('/')
        .next()
        .unwrap_or("document.pdf")
        .split('?')
        .next()
        .unwrap_or("document.pdf");

    let name = if base.ends_with(".pdf") || base.ends_with(".PDF") {
        base.to_string()
    } else {
        format!("{}.pdf", base)
    };

    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .take(200)
        .collect();

    if safe.len() < 3 {
        format!("{}_{}.pdf", &hash[..12], safe)
    } else {
        safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    async fn response(body: Vec<u8>) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await;
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/pdf\r\nConnection: close\r\n\r\n").await;
            let _ = stream.write_all(&body).await;
        });
        reqwest::Client::new()
            .get(format!("http://{address}/document.pdf"))
            .send()
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn oversized_response_without_content_length_is_rejected_and_cleaned() {
        // Restricted sandbox runs cannot exercise a real HTTP stream; CI/live checks run with networking.
        if tokio::net::TcpListener::bind("127.0.0.1:0").await.is_err() {
            return;
        }
        let directory =
            std::env::temp_dir().join(format!("ayla-download-test-{}", uuid::Uuid::new_v4()));
        let mut body = b"%PDF".to_vec();
        body.resize(1024 * 1024 + 1, b'x');
        let error = Downloader::new()
            .unwrap()
            .with_max_size(1)
            .save_response(
                "https://example.org/test.pdf",
                &directory,
                response(body).await,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("during download"));
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[tokio::test]
    async fn completed_download_is_atomic_and_content_hashed() {
        if tokio::net::TcpListener::bind("127.0.0.1:0").await.is_err() {
            return;
        }
        let directory =
            std::env::temp_dir().join(format!("ayla-download-test-{}", uuid::Uuid::new_v4()));
        let body = b"%PDF-1.4\nfixture".to_vec();
        let expected = format!("{:x}", Sha256::digest(&body));
        let result = Downloader::new()
            .unwrap()
            .save_response(
                "https://example.org/test.pdf",
                &directory,
                response(body.clone()).await,
            )
            .await
            .unwrap();
        assert_eq!(result.sha256, expected);
        assert_eq!(std::fs::read(&result.file_path).unwrap(), body);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
