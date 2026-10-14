use crate::types::OcrWarning;
use std::collections::{HashMap, HashSet};

const MIN_CONTENT_CHARS: usize = 60;
const MAX_CONTENT_CHARS: usize = 30_000;
const REPETITION_WINDOW: usize = 150;

pub struct QualityResult {
    pub text: String,
    pub warnings: Vec<OcrWarning>,
    pub retry_recommended: bool,
}

pub fn inspect(page: u16, raw: &str) -> QualityResult {
    let cleaned = cleanup(raw);
    let mut warnings = Vec::new();
    if cleaned.fence_removed {
        warnings.push(OcrWarning::page(
            page,
            "markdown_fence_removed",
            "Model-added Markdown fence markers were removed from the page text",
        ));
    }
    if cleaned.duplicate_removed {
        warnings.push(OcrWarning::page(
            page,
            "duplicate_content_removed",
            "Repeated OCR blocks were removed from the page text",
        ));
    }
    let mut text = if let Some(offset) = prompt_echo_offset(&cleaned.text) {
        warnings.push(OcrWarning::page(
            page,
            "prompt_echo_removed",
            "OCR instructions echoed by the model were removed from the page text",
        ));
        cleaned.text[..previous_char_boundary(&cleaned.text, offset)]
            .trim()
            .to_string()
    } else {
        cleaned.text
    };
    let mut retry_recommended = false;
    if let Some(offset) = repetition_offset(&text) {
        text.truncate(previous_char_boundary(&text, offset));
        text = text.trim().to_string();
        warnings.push(OcrWarning::page(
            page,
            "repetition_truncated",
            "A repeated model-output loop was removed",
        ));
        retry_recommended = true;
    }
    let mut short_lines = HashMap::new();
    for line in text.lines().map(str::trim) {
        if (12..32).contains(&line.chars().count()) {
            *short_lines.entry(line).or_insert(0usize) += 1;
        }
    }
    if short_lines.values().any(|count| *count > 1) {
        warnings.push(OcrWarning::page(
            page,
            "repeated_short_lines",
            "Some short lines repeat; compare them with the source page before approving OCR knowledge",
        ));
    }
    let chars = text.chars().count();
    if chars == 0 {
        warnings.push(OcrWarning::page(
            page,
            "empty_output",
            "The OCR model returned no usable text",
        ));
        retry_recommended = true;
    } else if chars < MIN_CONTENT_CHARS {
        warnings.push(OcrWarning::page(
            page,
            "short_output",
            format!("Only {chars} characters were recognized"),
        ));
        retry_recommended = true;
    } else if chars > MAX_CONTENT_CHARS {
        warnings.push(OcrWarning::page(
            page,
            "large_output",
            format!("The page produced an unusually large {chars}-character result"),
        ));
        retry_recommended = true;
    }
    QualityResult {
        text,
        warnings,
        retry_recommended,
    }
}

fn prompt_echo_offset(text: &str) -> Option<usize> {
    const MARKERS: [&str; 5] = [
        "This page has TWO columns.",
        "This page has a single column or no reliable embedded-text layout hint.",
        "Preserve all visible text, heading structure, exact numbers, and symbols.",
        "Output only Markdown content, without commentary or code fences.",
        "Convert document page ",
    ];
    MARKERS.iter().filter_map(|marker| text.find(marker)).min()
}

struct CleanupResult {
    text: String,
    fence_removed: bool,
    duplicate_removed: bool,
}

fn cleanup(raw: &str) -> CleanupResult {
    let trimmed = raw.trim();
    let decoded = trimmed
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&apos;", "'");
    let mut fence_removed = false;
    let without_fences = decoded
        .lines()
        .filter(|line| {
            let remove = is_model_markdown_fence(line);
            fence_removed |= remove;
            !remove
        })
        .collect::<Vec<_>>()
        .join("\n");
    let (text, duplicate_removed) = remove_duplicate_blocks(&without_fences);
    CleanupResult {
        text: text.trim().to_string(),
        fence_removed,
        duplicate_removed,
    }
}

fn is_model_markdown_fence(line: &str) -> bool {
    let trimmed = line.trim();
    let ticks = trimmed
        .chars()
        .take_while(|character| *character == '`')
        .count();
    if ticks < 3 {
        return false;
    }
    matches!(
        trimmed[ticks..].trim().to_ascii_lowercase().as_str(),
        "" | "md" | "markdown"
    )
}

fn remove_duplicate_blocks(text: &str) -> (String, bool) {
    let mut seen = HashSet::new();
    let mut removed = false;
    let mut blocks = Vec::new();
    for block in text.split("\n\n") {
        let normalized = block.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized.chars().count() >= 32 && !seen.insert(normalized) {
            removed = true;
            continue;
        }
        blocks.push(block.trim());
    }
    (blocks.join("\n\n"), removed)
}

fn previous_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn repetition_offset(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.len() < REPETITION_WINDOW * 3 {
        return None;
    }
    const BASE: u64 = 131;
    const MODULUS: u64 = 1_000_000_007;
    let mut hash = 0u64;
    let mut power = 1u64;
    for byte in bytes.iter().take(REPETITION_WINDOW) {
        hash = (hash * BASE + *byte as u64) % MODULUS;
        power = power * BASE % MODULUS;
    }
    let mut seen = HashMap::<u64, Vec<usize>>::new();
    for index in 0..=bytes.len() - REPETITION_WINDOW {
        if index > 0 {
            hash = (hash * BASE + MODULUS - bytes[index - 1] as u64 * power % MODULUS) % MODULUS;
            hash = (hash + bytes[index + REPETITION_WINDOW - 1] as u64) % MODULUS;
        }
        let positions = seen.entry(hash).or_default();
        let matches = positions
            .iter()
            .filter(|position| {
                bytes[**position..**position + REPETITION_WINDOW]
                    == bytes[index..index + REPETITION_WINDOW]
            })
            .count();
        if matches >= 2 {
            return Some(index);
        }
        if positions.len() < 4 {
            positions.push(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_outer_markdown_fence_and_entities() {
        let result = inspect(1, "```markdown\n# A &amp; B\n```\n");
        assert_eq!(result.text, "# A & B");
    }

    #[test]
    fn removes_malformed_fences_and_duplicate_blocks_from_incomplete_pages() {
        let input = "Fig 6: MLR-Forward Model (Marketing)\n\nFig 7: MLR-Full Model (Bank Dataset)\n``````\n```markdown\n\nJournal header with enough text to remain a useful OCR paragraph.\n\nFig 6: MLR-Forward Model (Marketing)\n\nFig 7: MLR-Full Model (Bank Dataset)";
        let result = inspect(7, input);
        assert!(!result.text.contains("```"));
        assert_eq!(
            result
                .text
                .matches("Fig 6: MLR-Forward Model (Marketing)")
                .count(),
            1
        );
        assert_eq!(
            result
                .text
                .matches("Fig 7: MLR-Full Model (Bank Dataset)")
                .count(),
            1
        );
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "markdown_fence_removed"));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "duplicate_content_removed"));
    }

    #[test]
    fn truncates_repetition_and_reports_it() {
        let repeated = "A sufficiently varied but intentionally repeated scientific paragraph with numbers 12345 and symbols alpha beta gamma. This sentence extends the window substantially. ";
        let input = format!("Unique introduction. {repeated}{repeated}{repeated}{repeated}");
        let result = inspect(7, &input);
        assert!(result.text.starts_with("Unique introduction"));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "repetition_truncated"));
    }

    #[test]
    fn short_output_requests_retry() {
        let result = inspect(2, "short");
        assert!(result.retry_recommended);
        assert_eq!(result.warnings[0].code, "short_output");
    }

    #[test]
    fn repeated_short_headings_are_reported_without_erasing_source_content() {
        let input = "English experiment\n\nThe first specimen has pressure 42.75 MPa at 23 degrees.\n\nEnglish experiment\n\nThe second specimen has pressure 18.20 MPa at 31 degrees.";
        let result = inspect(1, input);
        assert_eq!(result.text, input);
        assert!(!result.retry_recommended);
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "repeated_short_lines"));
    }

    #[test]
    fn removes_echoed_ocr_instructions() {
        let input = "Actual page content with a measured value of 42.75 MPa and enough detail to pass the minimum gate.\n\nThis page has a single column or no reliable embedded-text layout hint.\nPreserve all visible text, heading structure, exact numbers, and symbols.";
        let result = inspect(1, input);
        assert!(result.text.ends_with("minimum gate."));
        assert!(!result.text.contains("layout hint"));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == "prompt_echo_removed"));
    }
}
