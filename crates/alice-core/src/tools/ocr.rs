// ── OCR tool: subprocess wrapper for ocr-helper ────────────

use super::types::*;
use super::IngestTool;
use std::path::PathBuf;
use tokio::process::Command;

pub struct OcrTool {
    bin_path: PathBuf,
}

impl OcrTool {
    pub fn new() -> Self {
        Self {
            bin_path: Self::find_binary(),
        }
    }

    fn find_binary() -> PathBuf {
        // CARGO_MANIFEST_DIR = .../Ayla/crates/alice-core/
        // Go up to the workspace parent, then to the sibling helper project.
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let external = base.join("../../../ocr_helper_system/target/release/ocr-helper");
        if external.exists() {
            return external.canonicalize().unwrap_or(external);
        }
        PathBuf::from("ocr-helper")
    }
}

#[async_trait::async_trait]
impl IngestTool for OcrTool {
    fn name(&self) -> &str {
        "ocr"
    }

    fn description(&self) -> &str {
        "Extracts text from PDF documents using OCR."
    }

    async fn is_available(&self) -> bool {
        let ok = self.bin_path.exists();
        if !ok {
            log::warn!("OCR tool not found at {:?}. Build it: cd ../ocr_helper_system && cargo build --release", self.bin_path);
        }
        ok
    }

    async fn run(&self, _query: &str, file_path: Option<&str>) -> Result<IngestionResult, String> {
        let input = file_path.unwrap_or(_query);
        let output_stem = format!("/tmp/alice_ocr_{}", uuid::Uuid::new_v4());
        let output_md = format!("{}.md", output_stem);

        let output = Command::new(&self.bin_path)
            .args([
                "--input",
                input,
                "--output",
                &output_md,
                "--count",
                "0",
                "--no-images",
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to run ocr-helper: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ocr-helper failed: {}", stderr));
        }

        let full_text = tokio::fs::read_to_string(&output_md)
            .await
            .map_err(|e| format!("Failed to read OCR output: {}", e))?;

        Ok(IngestionResult {
            summary: format!("OCR extracted {} chars from '{}'", full_text.len(), input),
            entities: vec![],
            facts: vec![],
            relations: vec![],
            full_text,
            source: SourceInfo {
                tool: "ocr_helper".into(),
                input: input.into(),
                output_path: Some(output_md),
                metadata: serde_json::json!({"file": input}),
            },
            timestamp: chrono::Utc::now(),
        })
    }
}
