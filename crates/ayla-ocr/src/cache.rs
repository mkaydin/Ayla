//! Private, resumable per-page OCR cache.

use crate::ocr_client::PageOcrOutput;
use crate::types::OcrWarning;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const CACHE_SCHEMA_VERSION: u8 = 1;
const MAX_CACHE_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_CACHE_RECORD_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedPage {
    schema_version: u8,
    page: u16,
    text: String,
    warnings: Vec<OcrWarning>,
}

#[derive(Debug, Clone)]
pub(crate) struct PageCache {
    directory: PathBuf,
    pub key: String,
}

impl PageCache {
    pub fn open(root: &Path, pdf: &Path, option_signature: &str) -> Result<Self> {
        let mut digest = Sha256::new();
        // Bump when page text normalization or quality warnings change so older cached
        // model artifacts cannot bypass newer quality gates.
        digest.update(b"ayla-ocr-page-cache-v3\0");
        digest.update(option_signature.as_bytes());
        digest.update(b"\0");
        let mut file = std::fs::File::open(pdf)
            .with_context(|| format!("Failed to open OCR source {}", pdf.display()))?;
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .with_context(|| format!("Failed to fingerprint OCR source {}", pdf.display()))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        let key = format!("{:x}", digest.finalize());
        let directory = root.join(&key);
        std::fs::create_dir_all(&directory).with_context(|| {
            format!("Failed to create OCR page cache at {}", directory.display())
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { directory, key })
    }

    pub fn load(&self, page: u16) -> Result<Option<PageOcrOutput>> {
        let path = self.page_path(page);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("Failed to inspect {}", path.display()))
            }
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            anyhow::bail!("OCR cache record is not a regular file: {}", path.display());
        }
        if metadata.len() == 0 || metadata.len() > MAX_CACHE_RECORD_BYTES {
            anyhow::bail!("OCR cache record has an invalid size: {}", path.display());
        }
        let bytes = std::fs::read(&path)
            .with_context(|| format!("Failed to read OCR cache record {}", path.display()))?;
        let cached: CachedPage = serde_json::from_slice(&bytes)
            .with_context(|| format!("Invalid OCR cache record {}", path.display()))?;
        if cached.schema_version != CACHE_SCHEMA_VERSION || cached.page != page {
            anyhow::bail!("OCR cache record identity mismatch for page {page}");
        }
        Ok(Some(PageOcrOutput {
            text: cached.text,
            warnings: cached.warnings,
        }))
    }

    pub fn store(&self, page: u16, output: &PageOcrOutput) -> Result<()> {
        // Partial evidence is reviewable, but must not suppress a future attempt.
        if output
            .warnings
            .iter()
            .any(|warning| warning.code == "generation_incomplete")
        {
            return Ok(());
        }
        let record = CachedPage {
            schema_version: CACHE_SCHEMA_VERSION,
            page,
            text: output.text.clone(),
            warnings: output.warnings.clone(),
        };
        let bytes = serde_json::to_vec(&record).context("Failed to serialize OCR page cache")?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_CACHE_RECORD_BYTES {
            anyhow::bail!("OCR page {page} cache record exceeds the 8 MiB limit");
        }
        // Caches are disposable; never let repeated documents exhaust the disk.
        let root = self.directory.parent().context("Cache has no root")?;
        let mut total = 0u64;
        for (directory_index, directory) in std::fs::read_dir(root)?.enumerate() {
            if directory_index >= 10_000 {
                anyhow::bail!("OCR cache directory limit reached; clear old caches");
            }
            let directory = directory?;
            if !directory.file_type()?.is_dir() {
                continue;
            }
            for (record_index, entry) in std::fs::read_dir(directory.path())?.enumerate() {
                if record_index >= 5_001 {
                    anyhow::bail!("OCR cache record limit reached");
                }
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    total = total.saturating_add(entry.metadata()?.len());
                }
            }
        }
        let previous = std::fs::metadata(self.page_path(page))
            .map(|entry| entry.len())
            .unwrap_or(0);
        if total
            .saturating_sub(previous)
            .saturating_add(bytes.len() as u64)
            > MAX_CACHE_TOTAL_BYTES
        {
            anyhow::bail!("OCR resume cache reached its 2 GiB budget; clear old cache directories to resume caching");
        }
        atomic_private_write(&self.page_path(page), &bytes)
    }

    fn page_path(&self, page: u16) -> PathBuf {
        self.directory.join(format!("page-{page:05}.json"))
    }
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("json.{}.part", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("Failed to create OCR cache record {}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)
        .with_context(|| format!("Failed to commit OCR cache record {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_round_trip_is_page_scoped_and_private() {
        let temp =
            std::env::temp_dir().join(format!("ayla-ocr-cache-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let pdf = temp.join("fixture.pdf");
        std::fs::write(&pdf, b"fixture-pdf-bytes").unwrap();
        let cache = PageCache::open(&temp.join("cache"), &pdf, "model=a;dpi=200").unwrap();
        let output = PageOcrOutput {
            text: "Recognized page".into(),
            warnings: vec![OcrWarning::page(2, "fixture", "warning")],
        };
        cache.store(2, &output).unwrap();
        let loaded = cache.load(2).unwrap().unwrap();
        assert_eq!(loaded.text, output.text);
        assert!(cache.load(1).unwrap().is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(cache.page_path(2))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn incomplete_generation_is_not_cached_as_a_completed_page() {
        let temp =
            std::env::temp_dir().join(format!("ayla-ocr-cache-partial-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let pdf = temp.join("fixture.pdf");
        std::fs::write(&pdf, b"public fixture").unwrap();
        let cache = PageCache::open(&temp.join("cache"), &pdf, "model=a").unwrap();
        cache
            .store(
                1,
                &PageOcrOutput {
                    text: "Partial pressure 42.75 MPa".into(),
                    warnings: vec![crate::types::OcrWarning::page(
                        1,
                        "generation_incomplete",
                        "Public fixture",
                    )],
                },
            )
            .unwrap();
        assert!(cache.load(1).unwrap().is_none());
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn cache_key_changes_with_source_or_options() {
        let temp = std::env::temp_dir().join(format!("ayla-ocr-key-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).unwrap();
        let pdf = temp.join("fixture.pdf");
        std::fs::write(&pdf, b"first").unwrap();
        let first = PageCache::open(&temp.join("cache"), &pdf, "model=a").unwrap();
        let options = PageCache::open(&temp.join("cache"), &pdf, "model=b").unwrap();
        std::fs::write(&pdf, b"second").unwrap();
        let source = PageCache::open(&temp.join("cache"), &pdf, "model=a").unwrap();
        assert_ne!(first.key, options.key);
        assert_ne!(first.key, source.key);
        std::fs::remove_dir_all(temp).unwrap();
    }
}
