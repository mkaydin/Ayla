use crate::models::DownloadedPdf;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct Downloader {
    client: reqwest::Client,
    max_size_bytes: u64,
}

impl Downloader {
    pub fn new() -> Result<Self> {
        let mut default_headers = reqwest::header::HeaderMap::new();
        default_headers.insert(
            reqwest::header::ACCEPT,
            "application/pdf,application/octet-stream,*/*"
                .parse()
                .unwrap(),
        );
        default_headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            "en-US,en;q=0.9".parse().unwrap(),
        );

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .user_agent(
                "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .default_headers(default_headers)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            max_size_bytes: 100 * 1024 * 1024,
        })
    }

    pub fn with_max_size(mut self, max_size_mb: u64) -> Self {
        self.max_size_bytes = max_size_mb * 1024 * 1024;
        self
    }

    pub async fn download_pdf(&self, url: &str, dest_dir: &Path) -> Result<DownloadedPdf> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context(format!("Failed to download: {}", url))?;
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

        let bytes = response
            .bytes()
            .await
            .context("Failed to read response body")?;

        if bytes.len() as u64 > self.max_size_bytes {
            anyhow::bail!("PDF exceeds max size limit after download");
        }

        let actual_is_pdf = bytes.len() >= 4 && &bytes[..4] == b"%PDF";
        if !actual_is_pdf {
            anyhow::bail!(
                "Downloaded content is not a valid PDF (magic bytes: {:?})",
                &bytes[..4.min(bytes.len())]
            );
        }

        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let hash = format!("{:x}", hasher.finalize());

        let file_name = sanitize_filename(url, &hash);
        tokio::fs::create_dir_all(dest_dir)
            .await
            .context("Failed to create download directory")?;

        let file_path = dest_dir.join(&file_name);

        if file_path.exists() {
            info!("PDF already downloaded: {}", file_path.display());
            let metadata = tokio::fs::metadata(&file_path).await?;
            return Ok(DownloadedPdf {
                original_url: url.to_string(),
                file_path,
                sha256: hash,
                file_size_bytes: metadata.len(),
            });
        }

        let mut file = tokio::fs::File::create(&file_path)
            .await
            .context("Failed to create file")?;

        file.write_all(&bytes)
            .await
            .context("Failed to write PDF to disk")?;

        info!(
            "Downloaded PDF: {} -> {} ({} bytes, valid PDF)",
            url,
            file_path.display(),
            bytes.len()
        );

        Ok(DownloadedPdf {
            original_url: url.to_string(),
            file_path,
            sha256: hash,
            file_size_bytes: bytes.len() as u64,
        })
    }

    pub async fn download_parallel(
        &self,
        urls: &[String],
        dest_dir: &Path,
    ) -> Vec<Result<DownloadedPdf>> {
        let handles: Vec<_> = urls
            .iter()
            .map(|url| {
                let downloader = self.clone();
                let url = url.clone();
                let dest_dir = dest_dir.to_path_buf();
                tokio::spawn(async move { downloader.download_pdf(&url, &dest_dir).await })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!("Download task panicked: {}", e);
                    results.push(Err(anyhow::anyhow!("Task panicked: {}", e)));
                }
            }
        }
        results
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
