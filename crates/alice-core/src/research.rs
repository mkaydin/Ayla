//! Host-neutral OCR/search research orchestration.

use crate::db::surreal::SurrealDB;
use crate::memory::{safe_truncate, MemoryManager};
use crate::{BackendError, EventSink, MemoryConfig, ProcessRegistry, ResourceLocator};
use serde_json::Value;
use std::collections::HashSet;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

async fn connect_search_db(manager: &MemoryManager) -> Result<SurrealDB, String> {
    if !manager.is_db_ready() {
        return Err("DB offline: Meta Search history cannot be stored".into());
    }
    let config = &manager.config;
    SurrealDB::connect(
        &config.surrealdb_url,
        &config.surrealdb_ns,
        &config.surrealdb_db,
        &config.surrealdb_user,
        &config.surrealdb_pass,
    )
    .await
}

pub async fn write_general_knowledge_document(
    config: &MemoryConfig,
    query: &str,
    report: &Value,
) -> Result<String, String> {
    const MAX_OUTPUT_TOKENS: usize = 4_096;
    let evidence = crate::tools::meta_search::build_knowledge_document(query, report);
    let prompt = format!(
        "Write a standalone general-knowledge document about \"{query}\" from the source evidence below.\n\
         The document will be ingested into a knowledge graph, so:\n\
         - use descriptive Markdown headings;\n\
         - state canonical entity names explicitly;\n\
         - express relationships as clear factual sentences;\n\
         - distinguish consensus, disagreement, and uncertainty;\n\
         - preserve source URLs as provenance;\n\
         - do not mention these instructions or invent unsupported claims.\n\n\
         SOURCE EVIDENCE:\n{evidence}"
    );
    let num_ctx = config.ollama_context_for(prompt.len(), MAX_OUTPUT_TOKENS);
    log::info!(
        "Meta Search knowledge writer: input={} bytes, num_ctx={}",
        prompt.len(),
        num_ctx
    );
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|error| format!("Knowledge writer client: {error}"))?;
    let response = client
        .post(format!("{}/api/chat", config.ollama_url))
        .json(&serde_json::json!({
            "model": config.llm_model,
            "messages": [
                {
                    "role": "system",
                    "content": "You write precise, evidence-grounded reference documents optimized for entity and relation extraction."
                },
                {"role": "user", "content": prompt}
            ],
            "stream": false,
            "options": {
                "temperature": 0.15,
                "num_ctx": num_ctx,
                "num_predict": MAX_OUTPUT_TOKENS,
            }
        }))
        .send()
        .await
        .map_err(|error| format!("Knowledge writer LLM: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Knowledge writer response: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "Knowledge writer LLM HTTP {}: {}",
            status,
            safe_truncate(&body, 300)
        ));
    }
    let json: Value =
        serde_json::from_str(&body).map_err(|error| format!("Knowledge writer JSON: {error}"))?;
    let document = json
        .pointer("/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .trim_start_matches("```markdown")
        .trim_start_matches("```md")
        .trim_end_matches("```")
        .trim()
        .to_string();
    if document.is_empty() {
        return Err("Knowledge writer returned an empty document".into());
    }
    Ok(document)
}

fn remove_html_element(mut html: String, tag: &str) -> String {
    let opening = format!("<{tag}");
    let closing = format!("</{tag}>");
    loop {
        let lowercase = html.to_ascii_lowercase();
        let Some(start) = lowercase.find(&opening) else {
            break;
        };
        let Some(relative_end) = lowercase[start..].find(&closing) else {
            html.replace_range(start.., " ");
            break;
        };
        let end = start + relative_end + closing.len();
        html.replace_range(start..end, " ");
    }
    html
}

pub fn html_to_text(html: &str) -> String {
    let mut cleaned = html.to_string();
    for tag in ["script", "style", "noscript", "svg"] {
        cleaned = remove_html_element(cleaned, tag);
    }
    let mut text = String::with_capacity(cleaned.len());
    let mut inside_tag = false;
    for character in cleaned.chars() {
        match character {
            '<' => {
                inside_tag = true;
                text.push(' ');
            }
            '>' => {
                inside_tag = false;
                text.push(' ');
            }
            _ if !inside_tag => text.push(character),
            _ => {}
        }
    }
    text.replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace('\0', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn is_safe_public_http_url(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return false;
    }
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return match address {
            std::net::IpAddr::V4(ip) => {
                !(ip.is_private()
                    || ip.is_loopback()
                    || ip.is_link_local()
                    || ip.is_broadcast()
                    || ip.is_documentation()
                    || ip.is_unspecified())
            }
            std::net::IpAddr::V6(ip) => {
                !(ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local())
            }
        };
    }
    true
}

async fn fetch_knowledge_page(client: reqwest::Client, url: String) -> Option<String> {
    const MAX_PAGE_BYTES: usize = 768 * 1024;
    if !is_safe_public_http_url(&url) {
        return None;
    }
    let response = client
        .get(&url)
        .header("Range", format!("bytes=0-{}", MAX_PAGE_BYTES - 1))
        .send()
        .await
        .ok()?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_PAGE_BYTES as u64)
    {
        return None;
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !(content_type.contains("text/")
        || content_type.contains("application/json")
        || content_type.contains("application/xhtml"))
    {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    let raw = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_PAGE_BYTES)]);
    let text = if content_type.contains("html") {
        html_to_text(&raw)
    } else {
        raw.replace('\0', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    (text.len() >= 160).then(|| safe_truncate(&text, 8_000).to_string())
}

pub async fn gather_knowledge_with_searxng(
    config: &MemoryConfig,
    query: &str,
) -> Result<Value, String> {
    const MAX_SOURCES: usize = 12;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(18))
        .redirect(reqwest::redirect::Policy::limited(6))
        .user_agent("Alice-Knowledge-Gatherer/1.0")
        .build()
        .map_err(|error| format!("Knowledge search client: {error}"))?;
    let queries = [
        (query.to_string(), "general"),
        (format!("{query} overview key concepts"), "general"),
        (format!("{query} applications limitations"), "general"),
        (query.to_string(), "science"),
    ];
    let mut seen_urls = HashSet::new();
    let mut results = Vec::new();
    let mut response_errors = Vec::new();

    for (search_query, category) in &queries {
        let response = match client
            .get(format!(
                "{}/search",
                config.searxng_url.trim_end_matches('/')
            ))
            .query(&[
                ("q", search_query.as_str()),
                ("format", "json"),
                ("categories", *category),
                ("pageno", "1"),
                ("safesearch", "1"),
            ])
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                response_errors.push(error.to_string());
                continue;
            }
        };
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| format!("SearXNG response: {error}"))?;
        if !status.is_success() {
            response_errors.push(format!("HTTP {}: {}", status, safe_truncate(&body, 160)));
            continue;
        }
        let parsed: Value =
            serde_json::from_str(&body).map_err(|error| format!("SearXNG JSON: {error}"))?;
        for result in parsed
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = result.get("url").and_then(Value::as_str) else {
                continue;
            };
            if !is_safe_public_http_url(url)
                || url.to_ascii_lowercase().contains(".pdf")
                || !seen_urls.insert(url.to_string())
            {
                continue;
            }
            results.push(result.clone());
            if results.len() >= MAX_SOURCES {
                break;
            }
        }
        for infobox in parsed
            .get("infoboxes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let url = infobox
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .or_else(|| {
                    infobox
                        .get("urls")
                        .and_then(Value::as_array)
                        .and_then(|urls| urls.first())
                        .and_then(|url| url.get("url"))
                        .and_then(Value::as_str)
                });
            let Some(url) = url else {
                continue;
            };
            if !is_safe_public_http_url(url) || !seen_urls.insert(url.to_string()) {
                continue;
            }
            results.push(serde_json::json!({
                "title": infobox.get("infobox").cloned().unwrap_or_else(|| Value::String("Reference overview".into())),
                "url": url,
                "content": infobox.get("content").cloned().unwrap_or_default(),
                "engine": infobox.get("engine").cloned().unwrap_or_else(|| Value::String("infobox".into())),
                "score": 0.75,
                "category": category,
            }));
            if results.len() >= MAX_SOURCES {
                break;
            }
        }
        if results.len() >= MAX_SOURCES {
            break;
        }
    }
    if results.is_empty() {
        let detail = response_errors
            .last()
            .map(String::as_str)
            .unwrap_or("no results returned");
        return Err(format!("SearXNG knowledge search failed: {detail}"));
    }

    let mut page_texts = vec![None; results.len()];
    let mut tasks = tokio::task::JoinSet::new();
    for (index, result) in results.iter().enumerate() {
        let Some(url) = result
            .get("url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        let page_client = client.clone();
        tasks.spawn(async move { (index, fetch_knowledge_page(page_client, url).await) });
    }
    while let Some(joined) = tasks.join_next().await {
        if let Ok((index, text)) = joined {
            page_texts[index] = text;
        }
    }
    let pages_fetched = page_texts.iter().filter(|text| text.is_some()).count();
    let sources: Vec<Value> = results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            let snippet = result.get("content").and_then(Value::as_str).unwrap_or("");
            let evidence = page_texts[index]
                .as_deref()
                .filter(|text| !text.is_empty())
                .unwrap_or(snippet);
            let score = result
                .get("score")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| 1.0 / (index + 1) as f64)
                .clamp(0.0, 1.0);
            serde_json::json!({
                "source_url": result.get("url").cloned().unwrap_or_default(),
                "title": result.get("title").cloned().unwrap_or_else(|| Value::String("Untitled source".into())),
                "authors": "",
                "year": null,
                "abstract_text": safe_truncate(evidence, 8_000),
                "key_findings": [],
                "methodology": "",
                "datasets": [],
                "relevance_score": score,
                "relevance_rationale": format!(
                    "General web result gathered by {}.",
                    result.get("engine").and_then(Value::as_str).unwrap_or("SearXNG")
                ),
                "engine": result.get("engine").cloned().unwrap_or_default(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "session_id": uuid::Uuid::new_v4().to_string(),
        "query": query,
        "enriched_query": queries.iter()
            .map(|(search_query, category)| format!("{search_query} [{category}]"))
            .collect::<Vec<_>>().join(" | "),
        "total_results_found": seen_urls.len(),
        "pdfs_downloaded": 0,
        "pages_rendered": pages_fetched,
        "papers_analyzed": sources,
        "synthesis": {
            "query": query,
            "cross_cutting_themes": [],
            "consensus_findings": [],
            "conflicting_findings": [],
            "gaps_identified": [],
            "ranked_papers": [],
            "summary": format!(
                "Gathered {} distinct sources and fetched readable content from {} pages. The KB-ready document is generated from this evidence.",
                results.len(), pages_fetched
            ),
        },
        "download_dir": null,
        "search_provider": "searxng_general",
    }))
}

async fn collect_search_pdfs(report: &Value, search_id: &str) -> Result<Vec<Value>, String> {
    let Some(download_dir) = report.get("download_dir").and_then(Value::as_str) else {
        return Ok(Vec::new());
    };
    let mut directory = match tokio::fs::read_dir(download_dir).await {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "Cannot read Meta Search download directory: {error}"
            ))
        }
    };
    let mut paths = Vec::new();
    while let Some(entry) = directory
        .next_entry()
        .await
        .map_err(|error| format!("Cannot inventory downloaded papers: {error}"))?
    {
        let path = entry.path();
        let is_pdf = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
        if is_pdf
            && entry
                .file_type()
                .await
                .map(|kind| kind.is_file())
                .unwrap_or(false)
        {
            paths.push(path);
        }
    }
    paths.sort();
    let pdf_source_urls: Vec<&str> = report
        .get("papers_analyzed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|paper| paper.get("source_url").and_then(Value::as_str))
        .filter(|url| url.to_ascii_lowercase().contains(".pdf"))
        .collect();
    let total_paths = paths.len();
    let mut documents = Vec::with_capacity(total_paths);
    for (index, path) in paths.into_iter().enumerate() {
        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|error| format!("Cannot inspect downloaded paper: {error}"))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("paper.pdf")
            .to_string();
        let source_url = pdf_source_urls
            .iter()
            .find(|url| {
                url.rsplit('/')
                    .next()
                    .and_then(|segment| segment.split('?').next())
                    .is_some_and(|segment| segment == name)
            })
            .copied()
            .or_else(|| {
                (pdf_source_urls.len() == total_paths)
                    .then(|| pdf_source_urls.get(index).copied())
                    .flatten()
            })
            .unwrap_or("");
        documents.push(serde_json::json!({
            "document_id": uuid::Uuid::new_v4().to_string(),
            "search_id": search_id,
            "name": name,
            "path": path.to_string_lossy(),
            "source_url": source_url,
            "mime_type": "application/pdf",
            "size_bytes": metadata.len(),
            "status": "downloaded",
            "extraction_method": "",
            "chars": 0,
            "chunks": 0,
        }));
    }
    Ok(documents)
}

async fn persist_search_result(
    manager: &MemoryManager,
    search_id: &str,
    query: &str,
    mode: &str,
    report: &Value,
    knowledge_document: &str,
    documents: &[Value],
) -> Result<(), String> {
    let db = connect_search_db(manager).await?;
    db.ensure_table("search_session").await?;
    db.ensure_table("search_document").await?;
    let mut variables = serde_json::Map::new();
    variables.insert("search_id".into(), Value::String(search_id.to_string()));
    db.exec_with_vars(
        "DELETE search_document WHERE search_id = $search_id; DELETE search_session WHERE search_id = $search_id;",
        variables,
    )
    .await?;
    let now = chrono::Utc::now().to_rfc3339();
    db.insert(
        "search_session",
        serde_json::json!({
            "search_id": search_id,
            "query": query,
            "mode": mode,
            "status": "complete",
            "enriched_query": report.get("enriched_query").cloned().unwrap_or_default(),
            "total_results_found": report.get("total_results_found").cloned().unwrap_or_default(),
            "pdfs_downloaded": documents.len(),
            "pages_rendered": report.get("pages_rendered").cloned().unwrap_or_default(),
            "synthesis": report.get("synthesis").cloned().unwrap_or_default(),
            "papers": report.get("papers_analyzed").or_else(|| report.get("papers")).cloned().unwrap_or_else(|| Value::Array(Vec::new())),
            "knowledge_document": knowledge_document,
            "created_at": now,
            "updated_at": now,
            "knowledge_ingested": false,
        }),
    )
    .await?;
    for document in documents {
        let mut record = document.clone();
        if let Some(object) = record.as_object_mut() {
            object.insert("created_at".into(), Value::String(now.clone()));
            object.insert("updated_at".into(), Value::String(now.clone()));
            object.insert("extracted_text".into(), Value::String(String::new()));
        }
        db.insert("search_document", record).await?;
    }
    Ok(())
}

pub async fn run_search(
    manager: &MemoryManager,
    resources: &ResourceLocator,
    events: &dyn EventSink,
    processes: &ProcessRegistry,
    query: String,
    mode: Option<String>,
) -> Result<Value, BackendError> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(BackendError::new(
            "empty_search_topic",
            "Search topic cannot be empty",
        ));
    }
    let mode = if mode.as_deref() == Some("papers") {
        "papers"
    } else {
        "knowledge"
    };
    let pid = uuid::Uuid::new_v4().to_string();
    log::info!("Search [{}]: starting for '{}' ({})", pid, query, mode);

    let mut parsed = if mode == "knowledge" {
        let _ = events.emit("search-started", serde_json::json!({ "pid": pid }));
        // The sidecar request itself remains explicitly cancellable. Avoid a
        // second process-registry cancellation race here: search is a durable
        // background operation and renderer lifecycle must not report it as
        // user-cancelled.
        gather_knowledge_with_searxng(&manager.config, &query)
            .await
            .map_err(BackendError::from)?
    } else {
        let binary = resources.resolve_tool(
            "ALICE_SEARCH_BIN",
            "meta_search",
            "../../meta_search_Js_render/target/release/meta_search",
        );
        if !binary.exists() {
            return Err(BackendError::new(
                "search_binary_missing",
                "Meta search binary not found. Build: cd ../meta_search_Js_render && cargo build --release",
            ));
        }
        let child = Command::new(&binary)
            .args([
                "-j",
                "-n",
                "10",
                "--model",
                &manager.config.llm_model,
                "--ollama-url",
                &manager.config.ollama_url,
                "--searxng-url",
                &manager.config.searxng_url,
                "--lightpanda-url",
                "ws://127.0.0.1:9222",
                &query,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                BackendError::new("search_spawn", format!("Failed to start search: {error}"))
            })?;
        let cancel_rx = processes.register_child(pid.clone(), child).await;
        let _process_cleanup = processes.cleanup_on_drop(pid.clone());
        let _ = events.emit("search-started", serde_json::json!({ "pid": pid }));
        let stdout = processes
            .wait_child(&pid, None, cancel_rx)
            .await
            .map_err(BackendError::from)?;
        crate::tools::meta_search::parse_meta_search_stdout(&stdout).map_err(BackendError::from)?
    };
    let search_id = parsed
        .get("session_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let deterministic_document =
        crate::tools::meta_search::build_knowledge_document(&query, &parsed);
    let (knowledge_document, knowledge_generation) = if mode == "knowledge" {
        match write_general_knowledge_document(&manager.config, &query, &parsed).await {
            Ok(document) => (document, "llm"),
            Err(error) => {
                log::warn!(
                    "Meta Search knowledge writer failed; using structured fallback: {}",
                    error
                );
                (deterministic_document, "structured_fallback")
            }
        }
    } else {
        (deterministic_document, "structured_synthesis")
    };
    let documents = collect_search_pdfs(&parsed, &search_id)
        .await
        .map_err(BackendError::from)?;
    let persistence_error = persist_search_result(
        manager,
        &search_id,
        &query,
        mode,
        &parsed,
        &knowledge_document,
        &documents,
    )
    .await
    .err();

    parsed["search_id"] = Value::String(search_id);
    parsed["mode"] = Value::String(mode.into());
    parsed["knowledge_document"] = Value::String(knowledge_document);
    parsed["knowledge_generation"] = Value::String(knowledge_generation.into());
    parsed["documents"] = Value::Array(documents);
    parsed["persisted"] = Value::Bool(persistence_error.is_none());
    if let Some(error) = persistence_error {
        log::warn!("Search history persistence failed: {}", error);
        parsed["persistence_error"] = Value::String(error);
    }
    parsed["pid"] = Value::String(pid);
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knowledge_page_cleanup_removes_markup_and_active_content() {
        let html = format!(
            "<html><style>.hidden {{ display:none }}</style><script>alert('x')</script>\
             <body><h1>Reinforcement Learning</h1><p>Agents learn &amp; improve.{}Safely.</p></body></html>",
            '\0'
        );
        let text = html_to_text(&html);
        assert!(text.contains("Reinforcement Learning"));
        assert!(text.contains("Agents learn & improve. Safely."));
        assert!(!text.contains("alert"));
        assert!(!text.contains("display:none"));
        assert!(!text.contains('\0'));
    }

    #[test]
    fn knowledge_fetch_blocks_local_and_non_http_urls() {
        assert!(!is_safe_public_http_url("http://127.0.0.1/private"));
        assert!(!is_safe_public_http_url("http://localhost/private"));
        assert!(!is_safe_public_http_url("file:///etc/passwd"));
        assert!(is_safe_public_http_url("https://example.com/article"));
    }
}
