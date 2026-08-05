mod ocr_client;
mod output;
mod pdf_renderer;

use anyhow::{Context, Result};
use ocr_client::OcrClient;
use output::PageResult;
use pdf_renderer::{OutputFormat, RenderConfig};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_PROMPT: &str = "Extract all visible text from this document page. Output ONLY the text printed on the page. Do not add commentary, summaries, or additional content. Stop immediately after the last visible word.";

#[derive(Debug, Clone, Copy)]
pub enum ImageFormat {
    Png,
    Jpeg { quality: u8 },
}

#[derive(Debug, Clone)]
pub struct OcrOptions {
    pub model: String,
    pub ollama_url: String,
    pub prompt: String,
    pub dpi: f32,
    pub max_width: i32,
    pub image_format: ImageFormat,
    pub grayscale: bool,
    pub sharpen: bool,
    pub contrast: bool,
    pub num_gpu: i32,
    pub retries: u32,
    /// Zero disables the total HTTP timeout. Connection setup still times out.
    pub request_timeout_secs: u64,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            model: "deepseek-ocr".into(),
            ollama_url: "http://127.0.0.1:11434".into(),
            prompt: DEFAULT_PROMPT.into(),
            dpi: 200.0,
            max_width: 2400,
            image_format: ImageFormat::Png,
            grayscale: true,
            sharpen: true,
            contrast: false,
            num_gpu: 0,
            retries: 3,
            request_timeout_secs: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrProgress {
    pub stage: String,
    pub current_page: usize,
    pub total_pages: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrDocument {
    pub text: String,
    pub pages_processed: usize,
    pub pages_skipped: usize,
    pub total_pages: usize,
}

struct TemporaryWorkspace(PathBuf);

impl TemporaryWorkspace {
    fn create() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("ayla-ocr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create OCR workspace at {}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for TemporaryWorkspace {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Ayla OCR workspace cleanup failed: {error}");
            }
        }
    }
}

fn notify(progress: Option<&(dyn Fn(OcrProgress) + Send + Sync)>, update: OcrProgress) {
    if let Some(progress) = progress {
        progress(update);
    }
}

/// Render and OCR a PDF inside the current backend process.
///
/// Cancelling the owning async task drops this future and removes its temporary
/// render workspace; no helper executable or intermediate Markdown file is used.
pub async fn extract_pdf(
    input: impl AsRef<Path>,
    options: OcrOptions,
    progress: Option<&(dyn Fn(OcrProgress) + Send + Sync)>,
) -> Result<OcrDocument> {
    let input = input.as_ref().to_path_buf();
    let source_name = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document.pdf")
        .to_string();
    let workspace = TemporaryWorkspace::create()?;
    let images_dir = workspace.0.join("pages");
    notify(
        progress,
        OcrProgress {
            stage: "rendering".into(),
            current_page: 0,
            total_pages: 0,
            message: "Rendering PDF pages".into(),
        },
    );

    let render_config = RenderConfig {
        dpi: options.dpi,
        max_width: options.max_width,
        format: match options.image_format {
            ImageFormat::Png => OutputFormat::Png,
            ImageFormat::Jpeg { quality } => OutputFormat::Jpeg {
                quality: quality.clamp(1, 100),
            },
        },
        grayscale: options.grayscale,
        sharpen: options.sharpen,
        contrast: options.contrast,
    };
    let rendered = tokio::task::spawn_blocking(move || {
        pdf_renderer::render_pages(&input, 1, 0, &render_config, &images_dir)
    })
    .await
    .context("PDF rendering task failed")??;

    let total_pages = rendered.len();
    let client = OcrClient::new(
        options.model.clone(),
        options.prompt.clone(),
        options.ollama_url,
        options.num_gpu,
        options.retries.max(1),
        options.request_timeout_secs,
    )?;
    let mut results = Vec::<PageResult>::with_capacity(total_pages);
    let mut pages_skipped = 0usize;
    for (index, page) in rendered.iter().enumerate() {
        notify(
            progress,
            OcrProgress {
                stage: "recognizing".into(),
                current_page: index + 1,
                total_pages,
                message: format!("Recognizing page {} of {}", index + 1, total_pages),
            },
        );
        match client.ocr_page(&page.image_path).await {
            Ok(text) => results.push(PageResult {
                page_num: page.page_num,
                text,
                image_path: String::new(),
            }),
            Err(error) => {
                pages_skipped += 1;
                eprintln!("Ayla OCR skipped page {}: {error}", page.page_num);
            }
        }
    }

    if results.is_empty() {
        anyhow::bail!("All {total_pages} PDF pages failed OCR");
    }
    results.sort_by_key(|page| page.page_num);
    let text = output::build_markdown(
        &results,
        &source_name,
        &options.prompt,
        &options.model,
        false,
    );
    notify(
        progress,
        OcrProgress {
            stage: "complete".into(),
            current_page: results.len(),
            total_pages,
            message: format!("OCR complete: {} pages processed", results.len()),
        },
    );
    Ok(OcrDocument {
        text,
        pages_processed: results.len(),
        pages_skipped,
        total_pages,
    })
}
