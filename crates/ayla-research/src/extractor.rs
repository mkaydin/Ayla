use crate::models::{DownloadedPdf, ExtractedSource, SourceType};
use anyhow::{Context, Result};
use std::path::Path;
use tracing::{debug, warn};

pub fn extract_pdf_text(pdf: &DownloadedPdf, max_chars: usize) -> Result<ExtractedSource> {
    debug!("Extracting text from: {}", pdf.file_path.display());

    extract_with_lopdf(&pdf.file_path, &pdf.original_url, max_chars)
}

fn extract_with_lopdf(path: &Path, url: &str, max_chars: usize) -> Result<ExtractedSource> {
    use lopdf::Document;

    let doc = Document::load(path).context("Failed to load PDF with lopdf")?;

    let mut text = String::new();
    let page_count = doc.page_iter().count();

    for (page_num, _gen) in doc.page_iter() {
        match doc.extract_text(&[page_num]) {
            Ok(page_text) => {
                let cleaned = page_text
                    .chars()
                    .filter(|c| !c.is_control() || *c == '\n')
                    .collect::<String>();
                if !cleaned.trim().is_empty() {
                    text.push_str(&cleaned);
                    text.push('\n');
                }
            }
            Err(e) => {
                debug!("lopdf extraction failed for page {}: {}", page_num, e);
            }
        }
        if text.len() > max_chars {
            break;
        }
    }

    if text.trim().is_empty() || text.len() < 100 {
        warn!(
            "lopdf text extraction yielded {} chars for {} ({} pages), trying fallback",
            text.len(),
            path.display(),
            page_count
        );
        text = extract_raw_pdf_text(path).unwrap_or_default();
    }

    let trimmed = crate::truncate_utf8(&text, max_chars).to_string();

    let sha = calculate_file_sha(path)?;

    Ok(ExtractedSource {
        source_url: url.to_string(),
        source_type: SourceType::Pdf {
            file_path: path.to_path_buf(),
            sha256: sha,
        },
        text: clean_text(&trimmed),
        length_chars: trimmed.len(),
    })
}

fn extract_raw_pdf_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    let mut result = String::new();
    let len = bytes.len();
    let mut i = 0;

    while i < len.saturating_sub(2) {
        if bytes[i] == b'(' && (i == 0 || bytes[i - 1] != b'\\') {
            let start = i + 1;
            let mut depth = 1u32;
            i += 1;
            while i < len && depth > 0 {
                match bytes[i] {
                    b'(' if bytes[i - 1] != b'\\' => depth += 1,
                    b')' if bytes[i - 1] != b'\\' => depth -= 1,
                    b'\\' => {}
                    _ => {}
                }
                i += 1;
            }
            let end = i.saturating_sub(1);
            if end > start + 10 {
                if let Ok(s) = std::str::from_utf8(&bytes[start..end]) {
                    let is_likely_text = s.chars().filter(|c| c.is_alphabetic()).count() as f64
                        / s.len().max(1) as f64
                        > 0.4;
                    if is_likely_text && s.len() > 15 {
                        let cleaned = s
                            .replace("\\n", "\n")
                            .replace("\\r", "")
                            .replace("\\(", "(")
                            .replace("\\)", ")");
                        result.push_str(&cleaned);
                        result.push('\n');
                    }
                }
            }
        } else {
            i += 1;
        }
    }

    Ok(result)
}

pub fn extract_from_rendered(url: &str, text_content: &str) -> ExtractedSource {
    ExtractedSource {
        source_url: url.to_string(),
        source_type: SourceType::WebPage,
        text: clean_text(text_content),
        length_chars: text_content.len(),
    }
}

pub fn extract_pdf_text_fallback(
    path: &Path,
    url: &str,
    max_chars: usize,
) -> Result<ExtractedSource> {
    extract_with_lopdf(path, url, max_chars)
}

fn clean_text(text: &str) -> String {
    text.lines()
        .map(|line| line.trim())
        .filter(|line| {
            let alpha_ratio = line.chars().filter(|c| c.is_alphabetic()).count() as f64
                / line.len().max(1) as f64;
            !line.is_empty() && alpha_ratio > 0.3
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn calculate_file_sha(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn deeply_nested_pdf_objects_are_rejected_without_process_abort() {
        fn fixture(depth: usize) -> Vec<u8> {
            let mut pdf = "%PDF-1.4\n".to_string();
            let first = pdf.len();
            pdf.push_str(&format!(
                "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /X {}0{} >>\nendobj\n",
                "[".repeat(depth),
                "]".repeat(depth)
            ));
            let second = pdf.len();
            pdf.push_str("2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
            let xref = pdf.len();
            pdf.push_str(&format!("xref\n0 3\n0000000000 65535 f \n{first:010} 00000 n \n{second:010} 00000 n \ntrailer\n<< /Root 1 0 R /Size 3 >>\nstartxref\n{xref}\n%%EOF\n"));
            pdf.into_bytes()
        }
        assert!(lopdf::Document::load_mem(&fixture(8)).is_ok());
        let document = lopdf::Document::load_mem(&fixture(10_380));
        assert!(document.is_err() || document.unwrap().get_object((1, 0)).is_err());
    }
}
