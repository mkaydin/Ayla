// ── Quick Extract tool ─────────────────────────────────────
// Fast text extraction from .txt .md .json .docx .pdf (text layer)
// No OCR — reads text directly or extracts from file formats.

use super::types::*;
use super::IngestTool;
use std::path::{Path, PathBuf};

pub struct QuickExtractTool;

impl QuickExtractTool {
    pub fn new() -> Self { Self }
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
        "/etc/shadow", "/etc/gshadow", ".ssh/", "/.ssh/",
        ".gnupg/", "/.gnupg/", "id_rsa", "id_ecdsa", "id_ed25519",
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
    let ext = Path::new(&path).extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "txt" | "md" | "markdown" | "rs" | "js" | "py" | "toml" | "yaml" | "yml"
        | "html" | "css" | "xml" | "csv" | "log" => {
            std::fs::read_to_string(&path)
                .map_err(|e| format!("Read error: {}", e))
        }
        "json" => {
            let raw = std::fs::read_to_string(&path)
                .map_err(|e| format!("Read error: {}", e))?;
            let v: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| format!("JSON parse error: {}", e))?;
            Ok(serde_json::to_string_pretty(&v).unwrap_or(raw))
        }
        "pdf" => {
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("Read error: {}", e))?;
            pdf_extract::extract_text_from_mem(&bytes)
                .map_err(|e| format!("PDF text extract: {}", e))
        }
        "docx" => {
            extract_docx(&path)
        }
        _ => {
            // Try as plain text
            std::fs::read_to_string(&path)
                .map_err(|e| format!("Unsupported format (tried text read): {}", e))
        }
    }
}

fn extract_docx(path: &str) -> Result<String, String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Cannot open docx: {}", e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Invalid docx (zip): {}", e))?;
    let mut doc = archive.by_name("word/document.xml")
        .map_err(|_| "docx: missing word/document.xml".to_string())?;

    use quick_xml::Reader;
    use quick_xml::events::Event;
    use std::io::Read;

    let mut xml = String::new();
    doc.read_to_string(&mut xml)
        .map_err(|e| format!("docx read: {}", e))?;

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut text = String::new();
    let mut in_text = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:t" => in_text = true,
            Ok(Event::End(ref e)) if e.name().as_ref() == b"w:t" => in_text = false,
            Ok(Event::End(ref e)) if e.name().as_ref() == b"w:p" => text.push('\n'),
            Ok(Event::Text(ref e)) if in_text => {
                text.push_str(&e.unescape().unwrap_or_default());
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
    let mut chunks = vec![];
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut current = String::new();

    for para in paragraphs {
        let para = para.trim();
        if para.is_empty() { continue; }

        if current.len() + para.len() > max_chars && !current.is_empty() {
            chunks.push(current.trim().to_string());
            current = String::new();
        }
        if !current.is_empty() { current.push_str("\n\n"); }
        current.push_str(para);

        while current.len() > max_chars {
            let split_at = current[..max_chars].rfind(' ').unwrap_or(max_chars);
            let chunk = current[..split_at].trim().to_string();
            if !chunk.is_empty() { chunks.push(chunk); }
            current = current[split_at..].trim().to_string();
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }
    if chunks.is_empty() { chunks.push(text.to_string()); }
    chunks
}

#[async_trait::async_trait]
impl IngestTool for QuickExtractTool {
    fn name(&self) -> &str { "quick_extract" }

    fn description(&self) -> &str {
        "Fast text extraction from .txt, .md, .json, .docx, .pdf (text layer). No OCR."
    }

    async fn is_available(&self) -> bool { true }

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
            summary: format!("Quick-extracted {} chars from '{}'", full_text.len(), filename),
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

/// Public helper: chunk + embed text, returns (chunks, point_ids)
pub async fn embed_and_store_chunks(
    text: &str,
    filename: &str,
    embedder: &crate::memory::embeddings::Embedder,
    qdrant: &std::sync::Arc<crate::db::qdrant::QdrantDB>,
) -> Result<(Vec<String>, Vec<u64>), String> {
    let chunks = chunk_text(text, 1500);
    if chunks.is_empty() { return Ok((vec![], vec![])); }

    let mut point_ids = vec![];
    for (i, chunk) in chunks.iter().enumerate() {
        let vector = embedder.embed(chunk).await?;
        let id = crate::memory::document_store::DocumentStore::chunk_id(filename, i as u32);
        qdrant.upsert("documents", id, vector, chunk).await?;
        point_ids.push(id);
    }
    Ok((chunks, point_ids))
}
