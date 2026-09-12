use anyhow::Context;
use base64::Engine;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

use crate::quality;
use crate::types::OcrWarning;

#[derive(Clone)]
pub struct OcrClient {
    client: reqwest::Client,
    model: String,
    prompt_override: Option<String>,
    ollama_url: String,
    num_gpu: i32,
    num_ctx: u32,
    max_retries: u32,
}

pub struct PageOcrOutput {
    pub text: String,
    pub warnings: Vec<OcrWarning>,
}

impl OcrClient {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        model: String,
        prompt: String,
        ollama_url: String,
        num_gpu: i32,
        num_ctx: u32,
        max_retries: u32,
        request_timeout_secs: u64,
    ) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
        let builder = if request_timeout_secs == 0 {
            builder
        } else {
            builder.timeout(Duration::from_secs(request_timeout_secs))
        };
        let prompt_override =
            (!prompt.trim().is_empty() && prompt != crate::DEFAULT_PROMPT).then_some(prompt);
        Ok(Self {
            client: builder.build().context("Failed to build HTTP client")?,
            model,
            prompt_override,
            ollama_url: ollama_url.trim_end_matches('/').to_string(),
            num_gpu,
            num_ctx,
            max_retries: max_retries.max(1),
        })
    }

    pub async fn ocr_page(
        &self,
        image_path: &Path,
        page: u16,
        columns: u8,
    ) -> anyhow::Result<PageOcrOutput> {
        let image_bytes = std::fs::read(image_path)
            .with_context(|| format!("Failed to read rendered page {}", image_path.display()))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(image_bytes);
        let primary_prompt = self
            .prompt_override
            .clone()
            .unwrap_or_else(|| page_prompt(page, columns));
        let mut last_error = None;
        for attempt in 1..=self.max_retries {
            let prompt = if attempt == 1 {
                primary_prompt.as_str()
            } else {
                retry_prompt(columns)
            };
            match self.generate(prompt, &encoded).await {
                Ok((raw, incomplete)) => {
                    let inspected = quality::inspect(page, &raw);
                    if should_retry_generation(
                        incomplete,
                        inspected.retry_recommended,
                        attempt,
                        self.max_retries,
                    ) {
                        last_error = Some(anyhow::anyhow!(
                            "page {page} returned {}OCR output{}",
                            if incomplete { "incomplete " } else { "" },
                            if inspected.warnings.is_empty() {
                                String::new()
                            } else {
                                format!(
                                    " ({})",
                                    inspected
                                        .warnings
                                        .iter()
                                        .map(|warning| warning.code.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            }
                        ));
                        continue;
                    }
                    let mut warnings = inspected.warnings;
                    if incomplete {
                        warnings.push(OcrWarning::page(
                            page,
                            "generation_incomplete",
                            "Ollama marked the page generation incomplete; usable text was preserved",
                        ));
                    }
                    return Ok(PageOcrOutput {
                        text: inspected.text,
                        warnings,
                    });
                }
                Err(error) if attempt < self.max_retries && is_retryable(&error) => {
                    last_error = Some(error);
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt.min(5)))).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("OCR page {page} failed")))
    }

    async fn generate(&self, prompt: &str, encoded_image: &str) -> anyhow::Result<(String, bool)> {
        let mut options = serde_json::json!({
            "temperature": 0.0,
            "num_ctx": self.num_ctx,
            "num_predict": 4096,
        });
        if self.num_gpu >= 0 {
            options["num_gpu"] = serde_json::json!(self.num_gpu);
        }
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "images": [encoded_image],
            "stream": false,
            "options": options,
            "keep_alive": "30m",
        });
        let response = self
            .client
            .post(format!("{}/api/generate", self.ollama_url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to reach Ollama at {}", self.ollama_url))?;
        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("Failed to read Ollama response")?;
        if !status.is_success() {
            anyhow::bail!(
                "Ollama HTTP {}: {}",
                status,
                response_body.chars().take(500).collect::<String>()
            );
        }
        let response: Value =
            serde_json::from_str(&response_body).context("Failed to parse Ollama response")?;
        if let Some(error) = response.get("error").and_then(Value::as_str) {
            anyhow::bail!("Ollama error: {error}");
        }
        let incomplete = response.get("done").and_then(Value::as_bool) == Some(false);
        let text = response
            .get("response")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if text.trim().is_empty() && incomplete {
            anyhow::bail!("Ollama returned an incomplete OCR response without text");
        }
        Ok((text, incomplete))
    }
}

fn should_retry_generation(
    incomplete: bool,
    quality_retry: bool,
    attempt: u32,
    max_retries: u32,
) -> bool {
    (incomplete || quality_retry) && attempt < max_retries
}

fn page_prompt(page: u16, columns: u8) -> String {
    let layout = if columns >= 2 {
        "This page has TWO columns. Read the complete left column from top to bottom, then the complete right column. Do not interleave columns."
    } else {
        "This page has a single column or no reliable embedded-text layout hint."
    };
    format!(
        "Convert document page {page} to Markdown.\n{layout}\n\
         Preserve all visible text, heading structure, exact numbers, and symbols. \
         Preserve tables as Markdown pipe tables. Preserve mathematics as LaTeX using $...$ \
         or $$...$$. Skip running headers, footers, and standalone page numbers. \
         Output only Markdown content, without commentary or code fences."
    )
}

fn retry_prompt(columns: u8) -> &'static str {
    if columns >= 2 {
        "Transcribe this scientific document page to Markdown. Read the full left column first, then the full right column. Preserve tables, equations, numbers, and symbols exactly. Output only page content."
    } else {
        "Transcribe this scientific document page to Markdown. Preserve tables, equations, numbers, and symbols exactly. Output only page content."
    }
}

fn is_retryable(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    [
        "failed to reach",
        "tcp connect",
        "connection refused",
        "timeout",
        "dns",
        "http 429",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_column_prompt_has_unambiguous_order() {
        let prompt = page_prompt(4, 2);
        assert!(prompt.contains("left column"));
        assert!(prompt.contains("right column"));
        assert!(prompt.contains("Do not interleave"));
    }

    #[test]
    fn retryable_errors_are_bounded_to_transport_and_server_failures() {
        assert!(is_retryable(&anyhow::anyhow!("Ollama HTTP 503")));
        assert!(!is_retryable(&anyhow::anyhow!("bad Ollama response")));
    }

    #[test]
    fn incomplete_generation_retries_while_an_attempt_remains() {
        assert!(should_retry_generation(true, false, 1, 2));
        assert!(should_retry_generation(false, true, 1, 2));
        assert!(!should_retry_generation(true, false, 2, 2));
        assert!(!should_retry_generation(false, false, 1, 2));
    }
}
