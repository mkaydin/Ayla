mod cache;
mod geometry;
mod knowledge;
mod ocr_client;
mod output;
mod pdf_renderer;
mod quality;
mod semantic;
mod types;

#[cfg(test)]
mod advanced_tests;

use anyhow::{Context, Result};
use ocr_client::{OcrClient, PageOcrOutput};
use output::PageResult;
use pdf_renderer::{OutputFormat, RenderConfig, RenderResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Instant;

pub use types::{
    FigureAsset, LayoutSource, OcrKnowledgeEntity, OcrKnowledgeMode, OcrKnowledgePackage,
    OcrKnowledgeRelation, OcrModels, OcrPageResult, OcrPerformanceMetrics, OcrWarning, PageLayout,
};

pub const DEFAULT_PROMPT: &str = "";

#[derive(Debug, Clone, Copy)]
pub enum ImageFormat {
    Png,
    Jpeg { quality: u8 },
}

#[derive(Debug, Clone)]
pub struct OcrOptions {
    pub model: String,
    pub model_revision: Option<String>,
    pub pdfium_library: Option<PathBuf>,
    pub ollama_url: String,
    pub prompt: String,
    pub dpi: f32,
    pub max_width: i32,
    pub image_format: ImageFormat,
    pub grayscale: bool,
    pub sharpen: bool,
    pub contrast: bool,
    pub num_gpu: i32,
    pub num_ctx: u32,
    pub parallel_pages: usize,
    pub retries: u32,
    /// Zero disables the total HTTP timeout. Connection setup still times out.
    pub request_timeout_secs: u64,
    pub resume_cache: bool,
    pub cache_dir: Option<PathBuf>,
    pub knowledge_mode: OcrKnowledgeMode,
    pub assembly_model: Option<String>,
    /// Explicit consent for semantic assembly models tagged `:cloud`.
    pub assembly_allow_cloud: bool,
}

impl Default for OcrOptions {
    fn default() -> Self {
        Self {
            model: "glm-ocr:latest".into(),
            model_revision: None,
            pdfium_library: None,
            ollama_url: "http://127.0.0.1:11434".into(),
            prompt: DEFAULT_PROMPT.into(),
            dpi: 200.0,
            max_width: 2400,
            image_format: ImageFormat::Png,
            grayscale: true,
            sharpen: true,
            contrast: false,
            // Negative means "let Ollama choose". Explicit zero disables GPU layers.
            num_gpu: -1,
            num_ctx: 8_192,
            parallel_pages: 1,
            retries: 3,
            request_timeout_secs: 0,
            resume_cache: false,
            cache_dir: None,
            knowledge_mode: OcrKnowledgeMode::Deterministic,
            assembly_model: None,
            assembly_allow_cloud: false,
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
    pub source_name: String,
    pub pages_processed: usize,
    pub pages_skipped: usize,
    pub total_pages: usize,
    pub page_results: Vec<OcrPageResult>,
    pub models: OcrModels,
    pub warnings: Vec<OcrWarning>,
    pub knowledge: OcrKnowledgePackage,
    pub knowledge_mode: OcrKnowledgeMode,
    pub performance: OcrPerformanceMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
}

struct TemporaryWorkspace(PathBuf);
struct RenderCancellation(Arc<AtomicBool>);
impl Drop for RenderCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl TemporaryWorkspace {
    fn create() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("ayla-ocr-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create OCR workspace at {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
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
    let started = Instant::now();
    let input = input.as_ref().to_path_buf();
    let source_name = input
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("document.pdf")
        .to_string();
    if std::fs::metadata(&input)?.len() > 100 * 1024 * 1024 {
        anyhow::bail!("OCR PDF exceeds the 100 MiB input limit");
    }
    let workspace = Arc::new(TemporaryWorkspace::create()?);
    let images_dir = workspace.0.join("pages");
    let page_cache = if options.resume_cache {
        let root = options
            .cache_dir
            .as_deref()
            .context("OCR resume caching is enabled but no cache directory was configured")?
            .to_path_buf();
        let source = input.clone();
        let signature = format!(
            "{};revision={}",
            cache_signature(&options),
            options.model_revision.as_deref().unwrap_or("unversioned")
        );
        Some(
            tokio::task::spawn_blocking(move || cache::PageCache::open(&root, &source, &signature))
                .await
                .context("OCR cache fingerprint task failed")??,
        )
    } else {
        None
    };
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
        library_path: options.pdfium_library.clone(),
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
    let render_input = input.clone();
    let cancel_render = RenderCancellation(Arc::new(AtomicBool::new(false)));
    let cancelled = cancel_render.0.clone();
    let render_workspace = workspace.clone();
    let rendered = tokio::task::spawn_blocking(move || {
        // Keep the directory alive until the native worker exits, including cancellation.
        let _workspace = render_workspace;
        pdf_renderer::render_pages_cancellable(
            &render_input,
            1,
            0,
            &render_config,
            &images_dir,
            &cancelled,
        )
    })
    .await
    .context("PDF rendering task failed")??;

    let total_pages = rendered.len();
    let peak_rendered_page_bytes = rendered
        .iter()
        .filter_map(|page| std::fs::metadata(&page.image_path).ok())
        .map(|metadata| metadata.len())
        .max()
        .unwrap_or(0);
    let client = OcrClient::new(
        options.model.clone(),
        options.prompt.clone(),
        options.ollama_url.clone(),
        options.num_gpu,
        options.num_ctx,
        options.retries.max(1),
        options.request_timeout_secs,
    )?;
    let mut results = Vec::<PageResult>::with_capacity(total_pages);
    let mut page_results = Vec::<OcrPageResult>::with_capacity(total_pages);
    let mut warnings = Vec::<OcrWarning>::new();
    let mut pending = Vec::with_capacity(total_pages);
    let mut pages_from_cache = 0usize;
    for page in rendered {
        let cached = match page_cache.as_ref() {
            Some(cache) => cache.load(page.page_num),
            None => Ok(None),
        };
        match cached {
            Ok(Some(recognized)) => {
                pages_from_cache += 1;
                let page_num = page.page_num;
                accept_page(
                    page,
                    recognized,
                    true,
                    &mut results,
                    &mut page_results,
                    &mut warnings,
                );
                notify(
                    progress,
                    OcrProgress {
                        stage: "cache".into(),
                        current_page: pages_from_cache,
                        total_pages,
                        message: format!("Restored page {page_num} from the resumable OCR cache"),
                    },
                );
            }
            Ok(None) => pending.push(page),
            Err(error) => {
                warnings.push(OcrWarning::page(
                    page.page_num,
                    "cache_record_invalid",
                    format!(
                        "Ignored an invalid OCR cache record for page {}: {error}",
                        page.page_num
                    ),
                ));
                pending.push(page);
            }
        }
    }
    let parallel_pages = options.parallel_pages.clamp(1, 8).min(total_pages.max(1));
    let mut tasks = tokio::task::JoinSet::new();
    let mut pending = VecDeque::from(pending);
    for _ in 0..parallel_pages {
        if let Some(page) = pending.pop_front() {
            tasks.spawn(recognize_page(client.clone(), page, page_cache.clone()));
        }
    }
    let mut completed = pages_from_cache;
    let mut pages_recognized = 0usize;
    while let Some(joined) = tasks.join_next().await {
        completed += 1;
        match joined.context("OCR page task failed")?? {
            (page, Ok(recognized), cache_error) => {
                let page_num = page.page_num;
                pages_recognized += 1;
                accept_page(
                    page,
                    recognized,
                    false,
                    &mut results,
                    &mut page_results,
                    &mut warnings,
                );
                if let Some(error) = cache_error {
                    warnings.push(OcrWarning::page(
                        page_num,
                        "cache_write_failed",
                        format!("Could not persist page {page_num} in the OCR cache: {error}"),
                    ));
                }
                notify(
                    progress,
                    OcrProgress {
                        stage: "recognizing".into(),
                        current_page: completed,
                        total_pages,
                        message: format!(
                            "Recognized page {page_num} ({completed} of {total_pages})"
                        ),
                    },
                );
            }
            (page, Err(error), _) => {
                let warning = OcrWarning::page(
                    page.page_num,
                    "page_failed",
                    format!("Page {} failed OCR: {error}", page.page_num),
                );
                eprintln!("Ayla OCR: {}", warning.message);
                warnings.push(warning);
                notify(
                    progress,
                    OcrProgress {
                        stage: "recognizing".into(),
                        current_page: completed,
                        total_pages,
                        message: format!(
                            "Skipped page {} ({completed} of {total_pages})",
                            page.page_num
                        ),
                    },
                );
            }
        }
        if let Some(page) = pending.pop_front() {
            tasks.spawn(recognize_page(client.clone(), page, page_cache.clone()));
        }
    }

    if results.is_empty() {
        let reasons = warnings
            .iter()
            .filter(|warning| warning.code == "page_failed")
            .map(|warning| warning.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::bail!("All {total_pages} PDF pages failed OCR: {reasons}");
    }
    results.sort_by_key(|page| page.page_num);
    page_results.sort_by_key(|page| page.page);
    notify(
        progress,
        OcrProgress {
            stage: "assembling".into(),
            current_page: results.len(),
            total_pages,
            message: "Merging recognized pages in document order".into(),
        },
    );
    let text = output::build_markdown(
        &results,
        &source_name,
        &options.prompt,
        &options.model,
        false,
    );
    let mut assembly_model = None;
    let mut knowledge = match options.knowledge_mode {
        OcrKnowledgeMode::Disabled => {
            knowledge::build_evidence_package(&results, &page_results, &source_name)
        }
        OcrKnowledgeMode::Deterministic | OcrKnowledgeMode::Semantic => {
            knowledge::build_knowledge_package(&results, &page_results, &source_name)
        }
    };
    if options.knowledge_mode == OcrKnowledgeMode::Semantic {
        notify(
            progress,
            OcrProgress {
                stage: "semantic_assembly".into(),
                current_page: results.len(),
                total_pages,
                message: "Assembling concepts, facts, summaries, and relationships with the configured Ollama model".into(),
            },
        );
        let model = options.assembly_model.as_deref().unwrap_or_default();
        let enrichment = semantic::SemanticAssembler::new(
            &options.ollama_url,
            model,
            options.num_ctx,
            options.request_timeout_secs,
            options.assembly_allow_cloud,
        );
        let enrichment = match enrichment {
            Ok(assembler) => assembler.enrich(&results, &mut knowledge).await,
            Err(error) => Err(error),
        };
        match enrichment {
            Ok(()) => assembly_model = Some(model.to_string()),
            Err(error) => {
                let message = format!(
                    "Semantic assembly failed; deterministic knowledge output was preserved: {error}"
                );
                knowledge.warnings.push(message.clone());
                warnings.push(OcrWarning {
                    code: "semantic_assembly_failed".into(),
                    message,
                    page: None,
                });
            }
        }
    }
    let recognized_characters = page_results.iter().map(|page| page.chars).sum();
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
        source_name,
        pages_processed: results.len(),
        pages_skipped: total_pages - results.len(),
        total_pages,
        page_results,
        models: OcrModels {
            ocr: options.model,
            revision: options.model_revision,
            assembly: assembly_model,
        },
        warnings,
        knowledge,
        knowledge_mode: options.knowledge_mode,
        performance: OcrPerformanceMetrics {
            elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            pages_from_cache,
            pages_recognized,
            peak_rendered_page_bytes,
            recognized_characters,
        },
        cache_key: page_cache.map(|cache| cache.key),
    })
}

async fn recognize_page(
    client: OcrClient,
    page: RenderResult,
    page_cache: Option<cache::PageCache>,
) -> Result<(RenderResult, Result<PageOcrOutput>, Option<anyhow::Error>)> {
    let result = client
        .ocr_page(&page.image_path, page.page_num, page.layout.columns)
        .await;
    let cache_error = result.as_ref().ok().and_then(|recognized| {
        page_cache
            .as_ref()
            .and_then(|cache| cache.store(page.page_num, recognized).err())
    });
    Ok((page, result, cache_error))
}

fn accept_page(
    page: RenderResult,
    recognized: PageOcrOutput,
    cached: bool,
    results: &mut Vec<PageResult>,
    page_results: &mut Vec<OcrPageResult>,
    warnings: &mut Vec<OcrWarning>,
) {
    let page_num = page.page_num;
    warnings.extend(recognized.warnings.iter().cloned());
    if page.layout.source == LayoutSource::None {
        warnings.push(OcrWarning::page(
            page_num,
            "layout_unavailable",
            "No embedded PDF text geometry was available; OCR used a one-column fallback",
        ));
    }
    page_results.push(OcrPageResult {
        page: page_num,
        chars: recognized.text.chars().count(),
        columns: page.layout.columns,
        headings: page.layout.headings,
        figures: page.layout.figures,
        layout_source: page.layout.source,
        warnings: recognized.warnings,
        cached,
    });
    results.push(PageResult {
        page_num,
        text: recognized.text,
        image_path: String::new(),
    });
}

fn cache_signature(options: &OcrOptions) -> String {
    let format = match options.image_format {
        ImageFormat::Png => "png".to_string(),
        ImageFormat::Jpeg { quality } => format!("jpeg:{}", quality.clamp(1, 100)),
    };
    let prompt_hash = format!("{:x}", Sha256::digest(options.prompt.as_bytes()));
    let endpoint_hash = format!("{:x}", Sha256::digest(options.ollama_url.as_bytes()));
    format!(
        "model={};endpoint={endpoint_hash};prompt={prompt_hash};dpi={:.3};max_width={};format={format};grayscale={};sharpen={};contrast={};num_gpu={};num_ctx={}",
        options.model,
        options.dpi,
        options.max_width,
        options.grayscale,
        options.sharpen,
        options.contrast,
        options.num_gpu,
        options.num_ctx,
    )
}
