// ── Quick Extract tool ─────────────────────────────────────
// Fast text extraction from .txt .md .json .docx .pdf (text layer)
// No OCR — reads text directly or extracts from file formats.

use super::types::*;
use super::IngestTool;
use std::path::{Path, PathBuf};
const MAX_EXTRACT_TEXT_BYTES: usize = 32 * 1024 * 1024;

pub struct QuickExtractTool;

impl QuickExtractTool {
    pub fn new() -> Self {
        Self
    }
}

/// Defense-in-depth check for paths that arrive from the frontend (issue R20). Rejects
/// directory traversal after normalization and a denylist of sensitive OS files.
///
/// This is not a full sandbox — the user may legitimately point the tool at files anywhere
/// on disk via the file dialog. The goal is to stop a compromised webview / injected LLM
/// output from silently reading `/etc/shadow` or `~/.ssh/id_rsa`.
pub fn validate_user_path(raw: &str) -> Result<PathBuf, String> {
    if raw.trim().is_empty() {
        return Err("Empty path".to_string());
    }
    let p = Path::new(raw);

    // Canonicalize if the target exists (resolves .., symlinks). Fall back to the raw path
    // so we still deny based on the raw string even for non-existent targets.
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());

    // Denylist — checked against BOTH raw and canonical forms so that traversal like
    // `/etc/../etc/shadow` (which canonicalizes to `/etc/shadow`) is still caught.
    const DENY: &[&str] = &[
        "/etc/shadow",
        "/etc/gshadow",
        ".ssh/",
        "/.ssh/",
        ".gnupg/",
        "/.gnupg/",
        "id_rsa",
        "id_ecdsa",
        "id_ed25519",
    ];
    let raw_lower = raw.to_lowercase();
    let canon_lower = canon.to_string_lossy().to_lowercase();
    for d in DENY {
        if raw_lower.contains(d) || canon_lower.contains(d) {
            return Err("Access to this path is not permitted".to_string());
        }
    }

    Ok(p.to_path_buf())
}

pub fn read_file_content(path: &str) -> Result<String, String> {
    let validated = validate_user_path(path)?;
    let path = validated.to_string_lossy().to_string();
    let ext = Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let maximum_input = if ext == "pdf" {
        100 * 1024 * 1024
    } else {
        MAX_EXTRACT_TEXT_BYTES
    };
    if std::fs::metadata(&path)
        .map_err(|error| error.to_string())?
        .len()
        > maximum_input as u64
    {
        return Err("Source file exceeds the extraction byte limit".into());
    }
    let result = match ext.as_str() {
        "txt" | "md" | "markdown" | "rs" | "js" | "py" | "toml" | "yaml" | "yml" | "html"
        | "css" | "xml" | "csv" | "log" => {
            std::fs::read_to_string(&path).map_err(|e| format!("Read error: {}", e))
        }
        "json" => {
            let raw = std::fs::read_to_string(&path).map_err(|e| format!("Read error: {}", e))?;
            let v: serde_json::Value =
                serde_json::from_str(&raw).map_err(|e| format!("JSON parse error: {}", e))?;
            Ok(serde_json::to_string_pretty(&v).unwrap_or(raw))
        }
        "pdf" => {
            let bytes = std::fs::read(&path).map_err(|e| format!("Read error: {}", e))?;
            crate::protocol_io::with_stdout_suppressed(|| {
                pdf_extract::extract_text_from_mem(&bytes)
            })
            .map_err(|e| format!("PDF text extract: {}", e))
        }
        "docx" => extract_docx(&path),
        _ => {
            // Try as plain text
            std::fs::read_to_string(&path)
                .map_err(|e| format!("Unsupported format (tried text read): {}", e))
        }
    }?;
    if result.len() > MAX_EXTRACT_TEXT_BYTES {
        return Err("Extracted text exceeds the 32 MiB limit".into());
    }
    Ok(result)
}

fn extract_docx(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open docx: {}", e))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid docx (zip): {}", e))?;
    let doc = archive
        .by_name("word/document.xml")
        .map_err(|_| "docx: missing word/document.xml".to_string())?;
    if doc.size() > MAX_EXTRACT_TEXT_BYTES as u64 {
        return Err("DOCX document XML exceeds the 32 MiB limit".into());
    }

    use quick_xml::events::Event;
    use quick_xml::Reader;
    use std::io::Read;

    let mut xml = String::new();
    doc.take(MAX_EXTRACT_TEXT_BYTES as u64 + 1)
        .read_to_string(&mut xml)
        .map_err(|e| format!("docx read: {}", e))?;
    if xml.len() > MAX_EXTRACT_TEXT_BYTES {
        return Err("DOCX document XML exceeds the 32 MiB limit".into());
    }

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut in_text = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:t" => in_text = true,
            Ok(Event::End(ref e)) if e.name().as_ref() == b"w:t" => in_text = false,
            Ok(Event::End(ref e)) if e.name().as_ref() == b"w:p" => text.push('\n'),
            Ok(Event::Text(ref e)) if in_text => {
                text.push_str(&e.xml10_content().map_err(|error| error.to_string())?);
            }
            Ok(Event::CData(ref e)) if in_text => {
                text.push_str(&e.decode().map_err(|error| error.to_string())?)
            }
            Ok(Event::GeneralRef(ref e)) if in_text => {
                if let Some(character) = e.resolve_char_ref().map_err(|error| error.to_string())? {
                    if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                        return Err("Illegal XML character reference".into());
                    }
                    text.push(character);
                } else {
                    let entity = e.decode().map_err(|error| error.to_string())?;
                    text.push_str(
                        quick_xml::escape::resolve_xml_entity(&entity)
                            .ok_or("Unsupported XML entity")?,
                    );
                }
            }
            Ok(Event::DocType(_)) => {
                return Err("DOCX document types/entities are unsupported".into())
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(text)
}

pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return if text.is_empty() {
            vec![]
        } else {
            vec![text.to_string()]
        };
    }
    let mut chunks = vec![];
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut current = String::new();

    for para in paragraphs {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }

        if current.len() + para.len() > max_chars && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(para);

        while current.len() > max_chars {
            let mut boundary = max_chars.min(current.len());
            while boundary > 0 && !current.is_char_boundary(boundary) {
                boundary -= 1;
            }
            if boundary == 0 {
                boundary = current.chars().next().map(char::len_utf8).unwrap_or(0);
            }
            let split_at = current[..boundary]
                .char_indices()
                .rev()
                .find(|(_, ch)| ch.is_whitespace())
                .map(|(idx, _)| idx)
                .filter(|idx| *idx > 0)
                .unwrap_or(boundary);
            let chunk = current[..split_at].trim().to_string();
            if !chunk.is_empty() {
                chunks.push(chunk);
            }
            current = current[split_at..].trim().to_string();
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    chunks
}

#[async_trait::async_trait]
impl IngestTool for QuickExtractTool {
    fn name(&self) -> &str {
        "quick_extract"
    }

    fn description(&self) -> &str {
        "Fast text extraction from .txt, .md, .json, .docx, .pdf (text layer). No OCR."
    }

    async fn is_available(&self) -> bool {
        true
    }

    async fn run(&self, _query: &str, file_path: Option<&str>) -> Result<IngestionResult, String> {
        let input = file_path.unwrap_or(_query);
        let filename = Path::new(input)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(input);

        log::info!("QuickExtract: reading '{}'", filename);
        let full_text = read_file_content(input)?;
        log::info!("QuickExtract: extracted {} chars", full_text.len());

        Ok(IngestionResult {
            summary: format!(
                "Quick-extracted {} chars from '{}'",
                full_text.len(),
                filename
            ),
            entities: vec![],
            facts: vec![],
            relations: vec![],
            full_text,
            source: SourceInfo {
                tool: "quick_extract".into(),
                input: input.into(),
                output_path: None,
                metadata: serde_json::json!({"file": input, "filename": filename}),
            },
            timestamp: chrono::Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{chunk_text, extract_docx, read_file_content, MAX_EXTRACT_TEXT_BYTES};
    use std::io::Write;

    #[test]
    fn born_digital_pdf_preserves_numbers_and_units() {
        let content = "BT /F1 14 Tf 72 720 Td (Measured pressure: 42.75 MPa.) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = "%PDF-1.4\n".to_string();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{object}\nendobj\n", index + 1));
        }
        let xref = pdf.len();
        pdf.push_str("xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets {
            pdf.push_str(&format!("{offset:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Root 1 0 R /Size 6 >>\nstartxref\n{xref}\n%%EOF\n"
        ));
        let path = std::env::temp_dir().join(format!("ayla-pdf-text-{}.pdf", uuid::Uuid::new_v4()));
        std::fs::write(&path, pdf).unwrap();
        let text = read_file_content(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        let text = text.unwrap();
        assert!(text.contains("42.75") && text.contains("MPa"));
    }

    #[test]
    fn docx_entities_preserve_spacing_and_unicode() {
        let path = std::env::temp_dir().join(format!("ayla-docx-{}.docx", uuid::Uuid::new_v4()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.write_all(b"<w:document><w:p><w:t>A &amp; B &#x130; &lt;tag&gt;<![CDATA[ literal & text]]></w:t></w:p></w:document>").unwrap();
        zip.finish().unwrap();
        let result = extract_docx(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        assert_eq!(result.unwrap(), "A & B İ <tag> literal & text\n");
    }

    #[test]
    fn compressed_docx_cannot_expand_past_the_text_budget() {
        let path =
            std::env::temp_dir().join(format!("ayla-docx-limit-{}.docx", uuid::Uuid::new_v4()));
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        zip.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
        let chunk = [b' '; 65536];
        for _ in 0..MAX_EXTRACT_TEXT_BYTES / chunk.len() {
            zip.write_all(&chunk).unwrap();
        }
        zip.write_all(b" ").unwrap();
        zip.finish().unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() < 1024 * 1024);
        let result = extract_docx(path.to_str().unwrap());
        std::fs::remove_file(path).unwrap();
        assert!(result.unwrap_err().contains("32 MiB"));
    }

    #[test]
    fn chunks_unicode_without_splitting_utf8() {
        let text = "İstanbul 😀 knowledge graph 日本語 memory";
        let chunks = chunk_text(text, 12);
        assert!(!chunks.is_empty());
        assert_eq!(chunks.join(" "), text);
    }
}
