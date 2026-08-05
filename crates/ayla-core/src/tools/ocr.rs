use super::types::*;
use super::IngestTool;

pub struct OcrTool;

impl OcrTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl IngestTool for OcrTool {
    fn name(&self) -> &str {
        "ocr"
    }

    fn description(&self) -> &str {
        "Extracts text from PDF documents with Ayla's in-process OCR engine."
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn run(&self, query: &str, file_path: Option<&str>) -> Result<IngestionResult, String> {
        let input = file_path.unwrap_or(query);
        let config = crate::MemoryConfig::default();
        let result = ayla_ocr::extract_pdf(
            input,
            ayla_ocr::OcrOptions {
                model: config.ocr_model,
                ollama_url: config.ollama_url,
                request_timeout_secs: config.ollama_long_timeout_secs as u64,
                ..Default::default()
            },
            None,
        )
        .await
        .map_err(|error| error.to_string())?;

        Ok(IngestionResult {
            summary: format!(
                "OCR extracted {} chars from '{}' across {} pages",
                result.text.len(),
                input,
                result.pages_processed
            ),
            entities: vec![],
            facts: vec![],
            relations: vec![],
            full_text: result.text,
            source: SourceInfo {
                tool: "ayla_ocr".into(),
                input: input.into(),
                output_path: None,
                metadata: serde_json::json!({
                    "file": input,
                    "pages_processed": result.pages_processed,
                    "pages_skipped": result.pages_skipped,
                }),
            },
            timestamp: chrono::Utc::now(),
        })
    }
}
