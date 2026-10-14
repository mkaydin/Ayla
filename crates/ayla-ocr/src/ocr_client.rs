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
        let mut recognition_fallback = false;
        for attempt in 1..=self.max_retries {
            let prompt = if recognition_fallback {
                // GLM's documented text task prompt:
                // https://github.com/zai-org/GLM-OCR/blob/main/glmocr/config.yaml
                "Text Recognition:"
            } else if attempt == 1 {
                primary_prompt.as_str()
            } else {
                retry_prompt(columns)
            };
            match self.generate(prompt, &encoded, recognition_fallback).await {
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
                    if recognition_fallback {
                        warnings.push(OcrWarning::page(
                            page,
                            "recognition_prompt_fallback",
                            "Used GLM's text-recognition prompt after a repeated-output failure; review reading order, tables, and equations",
                        ));
                    }
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
                    recognition_fallback |= use_recognition_fallback(&self.model, &error);
                    last_error = Some(error);
                    tokio::time::sleep(Duration::from_secs(2u64.pow(attempt.min(5)))).await;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("OCR page {page} failed")))
    }

    async fn generate(
        &self,
        prompt: &str,
        encoded_image: &str,
        recognition_fallback: bool,
    ) -> anyhow::Result<(String, bool)> {
        let mut options = serde_json::json!({
            "temperature": 0.0,
            "num_ctx": self.num_ctx,
            "num_predict": 4096,
        });
        if recognition_fallback {
            // Match GLM's documented recovery sampling and apply the repeat
            // penalty to the full bounded context, rather than just 64 tokens.
            options["top_k"] = serde_json::json!(1);
            options["top_p"] = serde_json::json!(0.00001);
            options["repeat_penalty"] = serde_json::json!(1.1);
            // Use a positive span: current Ollama runtimes reject the legacy
            // -1 sentinel for full-context repetition handling.
            options["repeat_last_n"] = serde_json::json!(self.num_ctx);
        }
        if self.num_gpu >= 0 {
            options["num_gpu"] = serde_json::json!(self.num_gpu);
        }
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "images": [encoded_image],
            "stream": true,
            "options": options,
            "keep_alive": "30m",
        });
        let mut response = self
            .client
            .post(format!("{}/api/generate", self.ollama_url))
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Failed to reach Ollama at {}", self.ollama_url))?;
        let status = response.status();
        if !status.is_success() {
            let response_body = response
                .text()
                .await
                .context("Failed to read Ollama response")?;
            anyhow::bail!(
                "Ollama HTTP {}: {}",
                status,
                response_body.chars().take(500).collect::<String>()
            );
        }
        let mut pending = Vec::new();
        let mut text = String::new();
        let mut received = 0usize;
        let mut inspected_at = 0usize;
        while let Some(chunk) = response
            .chunk()
            .await
            .context("Failed to read Ollama response")?
        {
            received += chunk.len();
            if received > 8 * 1024 * 1024 {
                anyhow::bail!("Ollama OCR response exceeds the 8 MiB limit");
            }
            pending.extend_from_slice(&chunk);
            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                let line = pending.drain(..=end).collect::<Vec<_>>();
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                if let Some(incomplete) = append_generation_line(&line, &mut text)? {
                    return finish_generation(text, incomplete);
                }
                if text.len().saturating_sub(inspected_at) >= 512 {
                    inspected_at = text.len();
                    if quality::inspect(0, &text)
                        .warnings
                        .iter()
                        .any(|warning| warning.code == "repetition_truncated")
                    {
                        // Dropping the response stops the looping inference. Existing
                        // page-quality retries and incomplete-output warnings still apply.
                        return finish_generation(text, true);
                    }
                }
            }
        }
        if !pending.iter().all(u8::is_ascii_whitespace) {
            if let Some(incomplete) = append_generation_line(&pending, &mut text)? {
                return finish_generation(text, incomplete);
            }
        }
        finish_generation(text, true)
    }
}

fn append_generation_line(line: &[u8], text: &mut String) -> anyhow::Result<Option<bool>> {
    let value: Value = serde_json::from_slice(line).context("Failed to parse Ollama response")?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        if text.trim().is_empty() {
            anyhow::bail!("Ollama error: {error}");
        }
        // Keep useful partial evidence; the caller flags incomplete generation.
        return Ok(Some(true));
    }
    if let Some(delta) = value.get("response").and_then(Value::as_str) {
        text.push_str(delta);
    }
    Ok((value.get("done").and_then(Value::as_bool) == Some(true)).then_some(false))
}

fn finish_generation(text: String, incomplete: bool) -> anyhow::Result<(String, bool)> {
    if text.trim().is_empty() && incomplete {
        anyhow::bail!("Ollama returned an incomplete OCR response without text");
    }
    Ok((text, incomplete))
}

fn use_recognition_fallback(model: &str, error: &anyhow::Error) -> bool {
    let name = model.split(':').next().unwrap_or(model);
    (name == "glm-ocr" || name.ends_with("/glm-ocr"))
        && error
            .to_string()
            .to_ascii_lowercase()
            .contains("token repeat limit reached")
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
        "token repeat limit reached",
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

    #[test]
    fn streamed_token_loop_preserves_partial_text_and_marks_it_incomplete() {
        let mut text = String::new();
        assert_eq!(
            append_generation_line(
                br#"{"response":"Pressure is 42.75 MPa.","done":false}"#,
                &mut text
            )
            .unwrap(),
            None
        );
        assert_eq!(
            append_generation_line(br#"{"error":"token repeat limit reached"}"#, &mut text)
                .unwrap(),
            Some(true)
        );
        let (text, incomplete) = finish_generation(text, true).unwrap();
        assert!(incomplete);
        assert_eq!(text, "Pressure is 42.75 MPa.");
        assert!(append_generation_line(
            br#"{"error":"token repeat limit reached"}"#,
            &mut String::new()
        )
        .is_err());
    }

    #[test]
    fn recognition_fallback_is_specific_to_glm_token_loops() {
        let repeated = anyhow::anyhow!("Ollama HTTP 500: token repeat limit reached");
        assert!(use_recognition_fallback("glm-ocr:latest", &repeated));
        assert!(!use_recognition_fallback("other-model:latest", &repeated));
        assert!(!use_recognition_fallback(
            "glm-ocr:latest",
            &anyhow::anyhow!("Ollama HTTP 503: service unavailable")
        ));
    }

    #[tokio::test]
    async fn glm_token_loop_recovers_with_bounded_native_prompt_retry() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut prompts = Vec::new();
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let (header_end, length) = loop {
                    let mut chunk = [0u8; 4096];
                    let size = socket.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&chunk[..size]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")?
                                    .trim()
                                    .parse::<usize>()
                                    .ok()
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while bytes.len() < header_end + length {
                    let mut chunk = [0u8; 4096];
                    let size = socket.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    bytes.extend_from_slice(&chunk[..size]);
                }
                let request: Value =
                    serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                if attempt == 0 {
                    assert!(request["options"].get("repeat_last_n").is_none());
                } else {
                    assert_eq!(request["options"]["repeat_last_n"], 8192);
                    assert_eq!(request["options"]["repeat_penalty"], 1.1);
                    assert_eq!(request["options"]["top_k"], 1);
                }
                prompts.push(request["prompt"].as_str().unwrap().to_string());
                let (status, body) = if attempt == 0 {
                    (
                        "500 Internal Server Error",
                        serde_json::json!({"error": "prediction aborted, token repeat limit reached"}),
                    )
                } else {
                    (
                        "200 OK",
                        serde_json::json!({"done": true, "response": "The public fixture records a pressure of 42.75 MPa and a temperature of 23 degrees."}),
                    )
                };
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
            prompts
        });
        let image = std::env::temp_dir().join(format!("ayla-ocr-retry-{}", uuid::Uuid::new_v4()));
        std::fs::write(&image, b"public mock image").unwrap();
        let client = OcrClient::new(
            "glm-ocr:latest".into(),
            crate::DEFAULT_PROMPT.into(),
            format!("http://{address}"),
            0,
            8192,
            2,
            5,
        )
        .unwrap();
        let result = client.ocr_page(&image, 1, 1).await.unwrap();
        std::fs::remove_file(image).unwrap();
        let prompts = server.await.unwrap();
        assert!(prompts[0].starts_with("Convert document page"));
        assert_eq!(prompts[1], "Text Recognition:");
        assert!(result.text.contains("42.75 MPa"));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "recognition_prompt_fallback"));
    }
}
