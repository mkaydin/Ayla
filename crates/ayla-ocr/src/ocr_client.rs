use anyhow::Context;
use base64::Engine;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

pub struct OcrClient {
    client: reqwest::Client,
    model: String,
    prompt: String,
    ollama_url: String,
    num_gpu: i32,
    max_retries: u32,
}

impl OcrClient {
    pub fn new(
        model: String,
        prompt: String,
        ollama_url: String,
        num_gpu: i32,
        max_retries: u32,
        request_timeout_secs: u64,
    ) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(15));
        let builder = if request_timeout_secs == 0 {
            builder
        } else {
            builder.timeout(Duration::from_secs(request_timeout_secs))
        };
        let client = builder.build().context("Failed to build HTTP client")?;

        let prompt = if prompt.is_empty() {
            crate::DEFAULT_PROMPT.to_string()
        } else {
            prompt
        };

        Ok(Self {
            client,
            model,
            prompt,
            ollama_url,
            num_gpu,
            max_retries,
        })
    }

    pub async fn ocr_page(&self, image_path: &Path) -> anyhow::Result<String> {
        let image_bytes = std::fs::read(image_path).with_context(|| "Failed to read image file")?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&image_bytes);

        let mut options = serde_json::json!({
            "temperature": 0.0,
            "num_predict": 2048,
        });

        options["num_gpu"] = serde_json::json!(self.num_gpu);

        let body = serde_json::json!({
            "model": self.model,
            "messages": [{
                "role": "user",
                "content": self.prompt,
                "images": [b64]
            }],
            "stream": false,
            "options": options,
        });

        let url = format!("{}/api/chat", self.ollama_url);

        for attempt in 1..=self.max_retries {
            match self.send_request(&url, &body).await {
                Ok(raw) => return Ok(clean_output(&raw)),
                Err(e) => {
                    let msg = e.to_string();
                    let is_connection_error = msg.contains("Failed to reach")
                        || msg.contains("tcp connect")
                        || msg.contains("connection refused")
                        || msg.contains("timeout")
                        || msg.contains("dns")
                        || msg.contains("HTTP 500")
                        || msg.contains("HTTP 502")
                        || msg.contains("HTTP 503")
                        || msg.contains("HTTP 504");

                    if is_connection_error && attempt < self.max_retries {
                        let wait = Duration::from_secs(2u64.pow(attempt));
                        eprintln!(
                            "  Connection error (attempt {attempt}/{}), retrying in {}s...",
                            self.max_retries,
                            wait.as_secs()
                        );
                        tokio::time::sleep(wait).await;
                        continue;
                    }

                    return Err(e);
                }
            }
        }

        unreachable!()
    }

    async fn send_request(&self, url: &str, body: &Value) -> anyhow::Result<String> {
        let response = self
            .client
            .post(url)
            .json(body)
            .send()
            .await
            .with_context(|| {
                format!(
                    "Failed to reach Ollama at {}. Is it running?",
                    self.ollama_url
                )
            })?;
        let status = response.status();
        let response_body = response
            .text()
            .await
            .context("Failed to read Ollama response")?;
        if !status.is_success() {
            let mut end = response_body.len().min(500);
            while end > 0 && !response_body.is_char_boundary(end) {
                end -= 1;
            }
            anyhow::bail!("Ollama HTTP {}: {}", status, &response_body[..end]);
        }
        let resp: Value = serde_json::from_str(&response_body)
            .context("Failed to parse Ollama response as JSON")?;

        if let Some(err) = resp["error"].as_str() {
            return Err(anyhow::anyhow!("Ollama error: {}", err));
        }

        if resp.get("done").and_then(|d| d.as_bool()) == Some(false) {
            return Err(anyhow::anyhow!("Ollama returned incomplete response"));
        }

        let text = resp["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        Ok(text)
    }
}

fn clean_output(text: &str) -> String {
    let special_patterns = [
        "<|im_start|>",
        "<|im_end|>",
        "<|end|>",
        "<|im_continue|>",
        "<|user|>",
        "<|assistant|>",
        "</angelscript>",
        "<|md_start|>",
        " response",
        "</table>",
        "<table>",
        "</assistant>",
        "Assistant:",
        "[End of image content]",
        "[End of example]",
        "<td rowspan",
        "<td colspan",
        "<tr>",
        "</tr>",
        "<th>",
        "</th>",
        "<h3>",
        "</h3>",
        "<h4>",
        "</h4>",
        "<br>",
        "<br/>",
        "<br />",
        "<p>",
        "</p>",
        "<th",
        "<td",
        "rowspan=",
        "colspan=",
        "valign=",
        " valign=",
        "=\">",
        "'>",
        "\">",
        " rowspan",
        " valign",
    ];

    let mut cleaned = text.to_string();

    for pattern in &special_patterns {
        cleaned = cleaned.replace(pattern, "");
    }

    cleaned = cleaned
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return true;
            }
            if trimmed.chars().all(|c| c.is_ascii_digit()) && trimmed.len() <= 3 {
                return false;
            }
            if trimmed
                .chars()
                .all(|c| c.is_ascii_digit() || c == '|' || c == ' ')
            {
                return false;
            }
            if trimmed == "---" {
                return false;
            }
            if trimmed.contains("<|") || trimmed.contains("</|") {
                return false;
            }
            if trimmed.contains("data:image") {
                return false;
            }
            let lower = trimmed.to_lowercase();
            if lower.contains("nonenone") || lower.starts_with("none") && lower.len() > 20 {
                return false;
            }
            if lower.starts_with("![") && lower.contains("](image)") {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n");

    let lines: Vec<&str> = dedup_consecutive(cleaned.lines().collect());
    let truncated = deduplicate_lines(&lines);

    let mut result = String::with_capacity(cleaned.len());
    let mut blank_run = 0usize;

    for &line in truncated.iter() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                result.push('\n');
            }
        } else {
            blank_run = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

fn dedup_consecutive(lines: Vec<&str>) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut last_nonempty: Option<&str> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.len() < 6 {
            out.push(line);
            continue;
        }
        if let Some(prev) = last_nonempty {
            if prev == trimmed {
                continue;
            }
            if trimmed.len() >= 20 && prev.len() > trimmed.len() && prev.contains(trimmed) {
                continue;
            }
            if prev.len() >= 20 && trimmed.len() > prev.len() && trimmed.contains(prev) {
                last_nonempty = Some(trimmed);
                out.pop();
                out.push(line);
                continue;
            }
        }
        last_nonempty = Some(trimmed);
        out.push(line);
    }
    out
}

fn deduplicate_lines<'a>(lines: &'a [&'a str]) -> &'a [&'a str] {
    use std::collections::HashMap;

    let mut counts: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.len() < 5 {
            continue;
        }
        if trimmed
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',' || c == ' ')
        {
            continue;
        }
        counts.entry(trimmed).or_default().push(i);
    }

    let mut cutoff: Option<usize> = None;

    for (_, positions) in &counts {
        let n = positions.len();
        if n < 2 {
            continue;
        }

        if let (Some(&first), Some(&second)) = (positions.first(), positions.get(1)) {
            let gap = second.saturating_sub(first);

            let cut_pos = if n >= 3 {
                let third = positions[2];
                let gap2 = third.saturating_sub(second);
                if gap >= 1 && (gap as isize - gap2 as isize).abs() <= gap as isize / 2 + 1 {
                    Some(second)
                } else {
                    None
                }
            } else if gap >= 2 && first > 0 {
                Some(first)
            } else {
                None
            };

            if let Some(c) = cut_pos {
                if c > 0 {
                    cutoff = Some(cutoff.map_or(c, |existing| existing.min(c)));
                }
            }
        }
    }

    if let Some(cut) = cutoff {
        return &lines[..cut];
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_special_tokens() {
        let input = "Hello\n<|im_end|>\nWorld\n<|end|>\n";
        assert_eq!(clean_output(input), "Hello\n\nWorld");
    }

    #[test]
    fn test_clean_quoted_lines() {
        let input = "0\n  <|im_end|>\n</angelscript>\n\nTitle";
        assert_eq!(clean_output(input), "Title");
    }

    #[test]
    fn test_collapse_blank_lines() {
        let input = "Line1\n\n\n\nLine2";
        assert_eq!(clean_output(input), "Line1\n\nLine2");
    }

    #[test]
    fn test_preserve_single_blank() {
        let input = "Line1\n\nLine2";
        assert_eq!(clean_output(input), "Line1\n\nLine2");
    }

    #[test]
    fn test_filter_single_digit_lines() {
        let input = "1\n\nActual text\n2";
        assert_eq!(clean_output(input), "Actual text");
    }

    #[test]
    fn test_deduplicate_repeated_lines() {
        let input = "LineA\nLineB\nRealX\nLineA\nLineB\nLineA\nLineB";
        let lines: Vec<&str> = input.lines().collect();
        let result = deduplicate_lines(&lines);
        assert_eq!(result, &["LineA", "LineB", "RealX"]);
    }

    #[test]
    fn test_deduplicate_double_repeat() {
        let input = "RealA123\nRealB456\nOther-extra\nDuplicateX here\nDuplicateY there\nMore stuff here\nDuplicateX here\nDuplicateY there";
        let lines: Vec<&str> = input.lines().collect();
        let result = deduplicate_lines(&lines);
        assert_eq!(result, &["RealA123", "RealB456", "Other-extra"]);
    }

    #[test]
    fn test_deduplicate_spaced_repeats() {
        let input =
            "Long title here\nAnother long line\nReal content here\nLong title here\nAnother long line\nLong title here\nAnother long line";
        let lines: Vec<&str> = input.lines().collect();
        let result = deduplicate_lines(&lines);
        assert_eq!(
            result,
            &["Long title here", "Another long line", "Real content here"]
        );
    }

    #[test]
    fn test_dedup_consecutive_substring() {
        let input = vec![
            "AAAA BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
            "BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
            "Real content here",
        ];
        let result = dedup_consecutive(input);
        assert_eq!(
            result,
            vec![
                "AAAA BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
                "Real content here"
            ]
        );
    }

    #[test]
    fn test_dedup_consecutive_substring_reverse() {
        let input = vec![
            "BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG",
            "AAAA BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
            "Real content here",
        ];
        let result = dedup_consecutive(input);
        assert_eq!(
            result,
            vec![
                "AAAA BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
                "Real content here"
            ]
        );
    }

    #[test]
    fn test_dedup_consecutive_blank_between() {
        let input = vec![
            "AAAA BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
            "",
            "BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
            "Real",
        ];
        let result = dedup_consecutive(input);
        assert_eq!(
            result,
            vec![
                "AAAA BBBBBB CCCCCC DDDDD EEEEE FFFFF GGGGG HHHHHH",
                "",
                "Real"
            ]
        );
    }
}
