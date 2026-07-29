// ── Tauri IPC commands for memory system ──────────────────

use tauri::{Emitter, State};
use crate::memory::MemoryManager;
use crate::processes::{ProcessEntry, ProcessRegistry};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::process::Command;
use std::collections::HashMap;

/// Maximum accepted payload size for text-ingestion commands (issue R21). 50 MiB.
const MAX_TEXT_PAYLOAD: usize = 50 * 1024 * 1024;
/// Maximum accepted messages-json payload for session save. 10 MiB.
const MAX_MESSAGES_PAYLOAD: usize = 10 * 1024 * 1024;

#[tauri::command]
pub async fn enrich_prompt(
    manager: State<'_, MemoryManager>,
    user_query: String,
    base_prompt: String,
) -> Result<String, String> {
    manager.enrich_context(&user_query, &base_prompt).await
}

#[tauri::command]
pub async fn process_turn(
    manager: State<'_, MemoryManager>,
    user_msg: String,
    assistant_msg: String,
) -> Result<(), String> {
    manager.process_turn(&user_msg, &assistant_msg).await
}

#[tauri::command]
pub async fn process_message(
    manager: State<'_, MemoryManager>,
    query: String,
    base_prompt: String,
) -> Result<serde_json::Value, String> {
    let (enriched_prompt, tool_status) = manager
        .process_with_tools(&query, &base_prompt)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "system_prompt": enriched_prompt,
        "tool_status": tool_status,
    }))
}

#[tauri::command]
pub async fn get_kb_data(
    manager: State<'_, MemoryManager>,
) -> Result<serde_json::Value, String> {
    let graph = manager.graph.lock().await;
    let nodes: Vec<serde_json::Value> = graph.all_nodes().iter().map(|n| {
        serde_json::json!({
            "name": n.name,
            "type": format!("{:?}", n.node_type),
            "source": n.source,
            "created": n.created,
        })
    }).collect();

    let edges: Vec<serde_json::Value> = graph.all_edges().iter().map(|(from, rel, to, conf)| {
        serde_json::json!({
            "from": from,
            "relation": rel,
            "to": to,
            "confidence": conf,
        })
    }).collect();

    Ok(serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "files": [],
    }))
}

#[tauri::command]
pub async fn kb_add_node(
    manager: State<'_, MemoryManager>,
    name: String,
    node_type: String,
) -> Result<(), String> {
    let nt = match node_type.as_str() {
        "Person" => crate::memory::graph::NodeType::Person,
        "Project" => crate::memory::graph::NodeType::Project,
        "Tool" => crate::memory::graph::NodeType::Tool,
        "Concept" => crate::memory::graph::NodeType::Concept,
        "Document" => crate::memory::graph::NodeType::Document,
        _ => crate::memory::graph::NodeType::Other,
    };
    let mut graph = manager.graph.lock().await;
    graph.upsert_node(&name, nt);
    graph.sync().await?;
    Ok(())
}

#[tauri::command]
pub async fn kb_delete_node(
    manager: State<'_, MemoryManager>,
    name: String,
) -> Result<(), String> {
    let mut graph = manager.graph.lock().await;
    graph.remove_node(&name);
    graph.sync().await?;
    Ok(())
}

#[tauri::command]
pub async fn kb_delete_edge(
    manager: State<'_, MemoryManager>,
    from: String,
    to: String,
    relation: String,
) -> Result<(), String> {
    let mut graph = manager.graph.lock().await;
    graph.remove_edge(&from, &to, &relation);
    graph.sync().await?;
    Ok(())
}

#[tauri::command]
pub async fn kb_add_edge(
    manager: State<'_, MemoryManager>,
    from: String,
    to: String,
    relation: String,
    confidence: f64,
) -> Result<(), String> {
    let mut graph = manager.graph.lock().await;
    let fi = graph.upsert_node(&from, crate::memory::graph::NodeType::Concept);
    let ti = graph.upsert_node(&to, crate::memory::graph::NodeType::Concept);
    graph.upsert_edge(fi, ti, &relation, confidence as f32);
    Ok(())
}

#[tauri::command]
pub async fn run_ocr(
    _manager: State<'_, MemoryManager>,
    procs: State<'_, ProcessRegistry>,
    file_path: String,
) -> Result<serde_json::Value, String> {
    // Defense-in-depth: validate the frontend-supplied path (issue R20).
    let validated = crate::tools::quick_extract::validate_user_path(&file_path)?;
    let file_path = validated.to_string_lossy().to_string();
    let pid = uuid::Uuid::new_v4().to_string();
    log::info!("OCR [{}]: starting for {}", pid, file_path);

    let bin_path = {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        base.join("../../ocr_helper_system/target/release/ocr-helper")
    };

    if !bin_path.exists() {
        return Err("OCR binary not found. Build: cd ../ocr_helper_system && cargo build --release".into());
    }

    let output_stem = format!("/tmp/alice_ocr_{}", uuid::Uuid::new_v4());
    let output_md = format!("{}.md", output_stem);

    let child = Command::new(&bin_path)
        .args(["--input", &file_path, "--output", &output_md, "--count", "0", "--no-images"])
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start OCR: {}", e))?;

    let procs_ref = procs.processes_ref();
    let cancel_rx = crate::processes::register(&procs_ref, pid.clone(), child).await;

    let text = wait_or_cancel(&procs_ref, &pid, &output_md, cancel_rx).await?;

    log::info!("OCR [{}]: done — {} chars", pid, text.len());
    Ok(serde_json::json!({
        "text": text,
        "pages": "?",
        "chars": text.len(),
        "pid": pid,
    }))
}

#[tauri::command]
pub async fn run_search(
    app_handle: tauri::AppHandle,
    _manager: State<'_, MemoryManager>,
    procs: State<'_, ProcessRegistry>,
    query: String,
) -> Result<serde_json::Value, String> {
    let pid = uuid::Uuid::new_v4().to_string();
    log::info!("Search [{}]: starting for '{}'", pid, query);

    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let bin_path = base.join("../../meta_search_Js_render/target/release/meta_search");

    if !bin_path.exists() {
        return Err("Meta search binary not found. Build: cd ../meta_search_Js_render && cargo build --release".into());
    }

    let child = Command::new(&bin_path)
        .args([
            "-j", "-n", "10",
            "--model", "gemma4:e4b",
            "--searxng-url", "http://127.0.0.1:8888",
            "--lightpanda-url", "ws://127.0.0.1:9222",
            &query,
        ])
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start search: {}", e))?;

    let procs_ref = procs.processes_ref();
    let cancel_rx = crate::processes::register(&procs_ref, pid.clone(), child).await;

    // Emit the pid immediately so the frontend's cancel button can act while the
    // search runs (issue H1 — previously the pid was only returned after completion,
    // making the cancel button a permanent no-op).
    let _ = app_handle.emit("search-started", serde_json::json!({ "pid": pid }));

    let stdout = wait_or_cancel(&procs_ref, &pid, "", cancel_rx).await?;

    let trimmed = stdout.trim();
    let json_start = trimmed.find('{').unwrap_or(0);
    let json_str = &trimmed[json_start..];

    let mut parsed: serde_json::Value = serde_json::from_str(json_str)
        .unwrap_or_else(|_| serde_json::json!({
            "papers_analyzed": [],
            "synthesis": {"summary": "Parse error"}
        }));
    parsed["pid"] = serde_json::json!(pid);

    Ok(parsed)
}

#[tauri::command]
pub async fn cancel_process(
    procs: State<'_, ProcessRegistry>,
    pid: String,
) -> Result<bool, String> {
    log::info!("Cancelling process {}", pid);
    let procs_ref = procs.processes_ref();
    Ok(crate::processes::kill(&procs_ref, &pid).await)
}

async fn wait_or_cancel(
    procs: &Arc<Mutex<HashMap<String, ProcessEntry>>>,
    pid: &str,
    output_path: &str,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<String, String> {
    // Take only the child out, LEAVING the entry (with its cancel_tx) in the map so that a
    // concurrent `kill()` can still fire cancel_rx to interrupt this wait (issue R7).
    // (Previously this called `remove()`, which dropped cancel_tx, causing cancel_rx to
    // resolve immediately and self-cancel every OCR/search.)
    let child = {
        let mut map = procs.lock().await;
        map.get_mut(pid)
            .and_then(|e| e.child.take())
    }.ok_or_else(|| "Cancelled".to_string())?;

    // Race completion against cancellation. If cancel wins, `wait_with_output` is dropped;
    // the Command is spawned with `.kill_on_drop(true)` (see run_ocr/run_search) so the
    // dropped Child is killed rather than orphaned.
    let output = tokio::select! {
        r = child.wait_with_output() => match r {
            Ok(o) => o,
            Err(e) => {
                procs.lock().await.remove(pid);
                return Err(format!("{}", e));
            }
        },
        _ = cancel_rx => {
            // kill() already removed the entry; clean up defensively in case it hadn't.
            procs.lock().await.remove(pid);
            return Err("Cancelled".to_string());
        }
    };

    // Child exited normally — remove the now-empty entry.
    procs.lock().await.remove(pid);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("{}", stderr.lines().last().unwrap_or("unknown")));
    }
    if output_path.is_empty() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        tokio::fs::read_to_string(output_path).await.map_err(|e| format!("{}", e))
    }
}

#[tauri::command]
pub async fn ingest_text(
    app_handle: tauri::AppHandle,
    manager: State<'_, MemoryManager>,
    text: String,
    source: String,
    source_type: String,
) -> Result<String, String> {
    if text.len() > MAX_TEXT_PAYLOAD {
        return Err(format!("Payload too large ({} bytes, max {})", text.len(), MAX_TEXT_PAYLOAD));
    }
    log::info!("Ingest: {} chars from '{}' (type: {})", text.len(), source, source_type);

    use crate::tools::types::{IngestionResult, SourceInfo};
    let result = IngestionResult {
        summary: format!("Ingested from {}: {}", source_type, &source[..source.len().min(60)]),
        entities: vec![],
        facts: vec![],
        relations: vec![],
        full_text: text,
        source: SourceInfo {
            tool: source_type.clone(),
            input: source.clone(),
            output_path: None,
            metadata: serde_json::json!({}),
        },
        timestamp: chrono::Utc::now(),
    };

    match &manager.ingestion {
        Some(ing) => {
            log::info!("Ingest: running ingestion pipeline...");
            {
                let mut store = manager.text_store.lock().await;
                store.insert(source.clone(), result.full_text.clone());
            }
            let mut ingestion = ing.lock().await;
            let msg = ingestion.ingest(result, Some(&app_handle)).await?;
            // Sync graph to DB
            {
                let mut graph = manager.graph.lock().await;
                let _ = graph.sync().await;
            }
            // Save text store to DB
            crate::memory::save_text_store(&manager).await;
            log::info!("Ingest: {}", msg);
            Ok(msg)
        }
        None => {
            log::info!("Ingest: DB offline, storing in graph and text store");
            let mut graph = manager.graph.lock().await;
            graph.upsert_node(&source, crate::memory::graph::NodeType::Document);
            let mut store = manager.text_store.lock().await;
            store.insert(source.clone(), result.full_text.clone());
            let node_count = graph.node_count();
            let edge_count = graph.edge_count();
            log::info!("Ingest: graph={} nodes/{} edges, text_store={} docs", node_count, edge_count, store.len());
            Ok(format!("Stored {} chars in memory. Graph: {} nodes, {} edges.",
                result.full_text.len(), node_count, edge_count))
        }
    }
}

#[tauri::command]
pub async fn run_consolidation(
    manager: State<'_, MemoryManager>,
) -> Result<serde_json::Value, String> {
    let report = manager.run_consolidation().await?;
    Ok(serde_json::json!({
        "merges": report.merges,
        "pruned": report.pruned,
        "reinforced": report.reinforced,
    }))
}

#[tauri::command]
pub async fn save_chat_session(
    manager: State<'_, MemoryManager>,
    session_id: String,
    title: String,
    messages_json: String,
) -> Result<(), String> {
    if !manager.db_ready { return Err("DB offline".into()); }
    if messages_json.len() > MAX_MESSAGES_PAYLOAD {
        return Err(format!("Session payload too large ({} bytes, max {})", messages_json.len(), MAX_MESSAGES_PAYLOAD));
    }
    use crate::db::surreal::SurrealDB;
    let db = SurrealDB::connect(&manager.config.surrealdb_url, &manager.config.surrealdb_ns, &manager.config.surrealdb_db, &manager.config.surrealdb_user, &manager.config.surrealdb_pass).await.map_err(|e| e)?;
    db.ensure_table("chat_session").await?;
    db.ensure_table("chat_message").await?;
    // Parameterized — session_id comes from the frontend (issue R3).
    let mut dv = serde_json::Map::new();
    dv.insert("sid".into(), serde_json::Value::String(session_id.clone()));
    let _ = db.exec_with_vars("DELETE chat_message WHERE session_id = $sid;", dv.clone()).await;
    let _ = db.exec_with_vars("DELETE chat_session WHERE id = $sid;", dv).await;
    db.insert("chat_session", serde_json::json!({"id": session_id, "title": title, "updated_at": chrono::Utc::now().to_rfc3339()})).await?;
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json).unwrap_or_default();
    for msg in messages {
        db.insert("chat_message", serde_json::json!({
            "session_id": session_id, "role": msg["role"], "content": msg["content"],
            "timestamp": chrono::Utc::now().to_rfc3339(),
        })).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn load_chat_sessions(
    manager: State<'_, MemoryManager>,
) -> Result<serde_json::Value, String> {
    if !manager.db_ready { return Ok(serde_json::json!([])); }
    use crate::db::surreal::SurrealDB;
    let db = SurrealDB::connect(&manager.config.surrealdb_url, &manager.config.surrealdb_ns, &manager.config.surrealdb_db, &manager.config.surrealdb_user, &manager.config.surrealdb_pass).await.map_err(|e| e)?;
    db.ensure_table("chat_session").await?;
    db.ensure_table("chat_message").await?;
    let sessions = db.query("SELECT * FROM chat_session ORDER BY updated_at DESC;").await?;
    let mut result = vec![];
    for s in sessions {
        let sid = s.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let msgs = {
            let mut mv = serde_json::Map::new();
            mv.insert("sid".into(), serde_json::Value::String(sid.to_string()));
            db.query_with_vars("SELECT * FROM chat_message WHERE session_id = $sid ORDER BY timestamp ASC;", mv).await?
        };
        let messages: Vec<serde_json::Value> = msgs.iter().map(|m| serde_json::json!({
            "role": m.get("role").and_then(|v| v.as_str()).unwrap_or(""),
            "content": m.get("content").and_then(|v| v.as_str()).unwrap_or(""),
        })).collect();
        result.push(serde_json::json!({"id": sid, "title": s.get("title").and_then(|v| v.as_str()).unwrap_or(""), "messages": messages}));
    }
    Ok(serde_json::json!(result))
}

#[tauri::command]
pub async fn delete_chat_session(
    manager: State<'_, MemoryManager>,
    session_id: String,
) -> Result<(), String> {
    if !manager.db_ready { return Err("DB offline".into()); }
    use crate::db::surreal::SurrealDB;
    let db = SurrealDB::connect(&manager.config.surrealdb_url, &manager.config.surrealdb_ns, &manager.config.surrealdb_db, &manager.config.surrealdb_user, &manager.config.surrealdb_pass).await.map_err(|e| e)?;
    db.ensure_table("chat_session").await?;
    db.ensure_table("chat_message").await?;
    let mut dv = serde_json::Map::new();
    dv.insert("sid".into(), serde_json::Value::String(session_id.clone()));
    let _ = db.exec_with_vars("DELETE chat_message WHERE session_id = $sid;", dv.clone()).await;
    let _ = db.exec_with_vars("DELETE chat_session WHERE id = $sid;", dv).await;
    Ok(())
}

#[tauri::command]
pub async fn check_db_ready(
    manager: State<'_, MemoryManager>,
) -> Result<bool, String> {
    Ok(manager.db_ready)
}

#[tauri::command]
pub async fn save_file_record(
    manager: State<'_, MemoryManager>,
    name: String, path: String, chars: u32, source_type: String, text: String,
) -> Result<(), String> {
    log::info!("File record: saving '{}' ({} chars, path={})", name, chars, &path[..path.len().min(40)]);
    if !manager.db_ready { return Err("DB offline".into()); }
    if text.len() > MAX_TEXT_PAYLOAD {
        return Err(format!("File text too large ({} bytes, max {})", text.len(), MAX_TEXT_PAYLOAD));
    }
    use crate::db::surreal::SurrealDB;
    let db = SurrealDB::connect(&manager.config.surrealdb_url, &manager.config.surrealdb_ns, &manager.config.surrealdb_db, &manager.config.surrealdb_user, &manager.config.surrealdb_pass).await.map_err(|e| e)?;
    db.ensure_table("kb_file").await?;
    let record = serde_json::json!({"name": name, "path": path, "chars": chars, "source": source_type, "date": chrono::Utc::now().to_rfc3339(), "text": text});
    db.insert("kb_file", record).await?;
    // Also persist into in-memory text store for backwards compat
    {
        let mut store = manager.text_store.lock().await;
        store.insert(name.clone(), text);
    }
    log::info!("File record: saved successfully");
    Ok(())
}

#[tauri::command]
pub async fn delete_file_record(
    manager: State<'_, MemoryManager>,
    name: String, delete_nodes: bool,
) -> Result<String, String> {
    if !manager.db_ready { return Err("DB offline".into()); }
    use crate::db::surreal::SurrealDB;
    let db = SurrealDB::connect(&manager.config.surrealdb_url, &manager.config.surrealdb_ns, &manager.config.surrealdb_db, &manager.config.surrealdb_user, &manager.config.surrealdb_pass).await.map_err(|e| e)?;
    db.ensure_table("kb_file").await?;
    // Parameterized — was a broken hand-rolled escape (`name.replace('\'', "\\'")` — issue R3).
    let mut nv = serde_json::Map::new();
    nv.insert("name".into(), serde_json::Value::String(name.clone()));
    let _ = db.exec_with_vars("DELETE kb_file WHERE name = $name;", nv).await?;
    {
        let mut store = manager.text_store.lock().await;
        store.remove(&name);
    }
    let mut removed_nodes = 0;
    if delete_nodes {
        let mut graph = manager.graph.lock().await;
        removed_nodes = graph.remove_by_source(&name);
        graph.sync().await?;
    }
    // Also delete the Qdrant document vectors (was never called — issue R18).
    if let Some(ref doc_store) = manager.document_store {
        if let Err(e) = doc_store.delete_by_filename(&name).await {
            log::warn!("Failed to delete vectors for '{}': {}", name, e);
        }
    }
    log::info!("File record: deleted '{}' ({} graph nodes removed)", name, removed_nodes);
    Ok(format!("Deleted '{}' ({} graph nodes removed)", name, removed_nodes))
}

#[tauri::command]
pub async fn delete_file_nodes(
    manager: State<'_, MemoryManager>,
    name: String,
) -> Result<String, String> {
    let mut graph = manager.graph.lock().await;
    let count = graph.remove_by_source(&name);
    graph.sync().await?;
    Ok(format!("Removed {} nodes from '{}'", count, name))
}

#[tauri::command]
pub async fn get_files(
    manager: State<'_, MemoryManager>,
) -> Result<serde_json::Value, String> {
    log::info!("Files: querying kb_file table");
    if !manager.db_ready { return Ok(serde_json::json!([])); }
    use crate::db::surreal::SurrealDB;
    let db = SurrealDB::connect(&manager.config.surrealdb_url, &manager.config.surrealdb_ns, &manager.config.surrealdb_db, &manager.config.surrealdb_user, &manager.config.surrealdb_pass).await.map_err(|e| e)?;
    db.ensure_table("kb_file").await?;
    let rows = db.query("SELECT * FROM kb_file ORDER BY date DESC;").await?;
    log::info!("Files: found {} records", rows.len());
    let files: Vec<serde_json::Value> = rows.iter().map(|r| serde_json::json!({
        "name": r.get("name").and_then(|v| v.as_str()).unwrap_or(""),
        "path": r.get("path").and_then(|v| v.as_str()).unwrap_or(""),
        "size": r.get("chars").and_then(|v| v.as_u64()).unwrap_or(0),
        "source": r.get("source").and_then(|v| v.as_str()).unwrap_or(""),
        "date": r.get("date").and_then(|v| v.as_str()).unwrap_or(""),
        "text": r.get("text").and_then(|v| v.as_str()).unwrap_or(""),
    })).collect();
    Ok(serde_json::json!(files))
}

#[tauri::command]
pub async fn get_document_text(
    manager: State<'_, MemoryManager>,
    name: String,
) -> Result<String, String> {
    let store = manager.text_store.lock().await;
    Ok(store.get(&name).cloned().unwrap_or_default())
}

#[tauri::command]
pub async fn run_quick_extract(
    manager: State<'_, MemoryManager>,
    file_path: String,
) -> Result<serde_json::Value, String> {
    use crate::tools::quick_extract;

    let filename = std::path::Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&file_path)
        .to_string();

    log::info!("QuickExtract: processing '{}'", filename);
    // Blocking file I/O + PDF/DOCX parsing off the async runtime worker (issue R22).
    let file_path_owned = file_path.clone();
    let full_text = tokio::task::spawn_blocking(move || quick_extract::read_file_content(&file_path_owned))
        .await
        .map_err(|e| format!("QuickExtract task panicked: {}", e))??;
    log::info!("QuickExtract: extracted {} chars from '{}'", full_text.len(), filename);

    let chunk_count = {
        let chunks = quick_extract::chunk_text(&full_text, 1500);
        let count = chunks.len();

        // Store chunks in Qdrant if available
        if let Some(ref doc_store) = manager.document_store {
            let _ = doc_store.store_chunks(&filename, &chunks).await;
        }
        count
    };

    Ok(serde_json::json!({
        "text": full_text,
        "filename": filename,
        "chars": full_text.len(),
        "chunks": chunk_count,
    }))
}

#[tauri::command]
pub async fn search_documents(
    manager: State<'_, MemoryManager>,
    query: String,
    limit: u32,
) -> Result<serde_json::Value, String> {
    let doc_store = manager.document_store.as_ref()
        .ok_or("Document store offline")?;
    let results = doc_store.search(&query, limit as u64).await?;
    Ok(serde_json::json!(results.iter().map(|(t, s)| serde_json::json!({
        "text": t,
        "score": s,
    })).collect::<Vec<_>>()))
}
