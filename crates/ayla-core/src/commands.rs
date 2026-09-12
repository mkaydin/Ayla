/// Protocol command catalog. Phase 2 uses this exact allowlist during the
/// Electron/Rust handshake.
pub const BACKEND_METHODS: &[&str] = &[
    "get_runtime_capabilities",
    "get_stt_status",
    "set_system_stt_enabled",
    "start_stt",
    "stt_push_audio",
    "stop_stt",
    "generate_tts",
    "stop_tts",
    "enrich_prompt",
    "process_turn",
    "process_message",
    "stream_chat",
    "get_kb_data",
    "get_graph_v2_data",
    "get_graph_v2_page",
    "migrate_legacy_graph_v2",
    "prune_graph_v2_backups",
    "analyze_and_merge_kb",
    "run_ocr",
    "ingest_ocr_knowledge",
    "approve_ocr_graph",
    "remove_ocr_graph",
    "rollback_graph_v2",
    "run_code_analysis",
    "get_code_analysis_runs",
    "get_code_analysis_run",
    "get_artifact_content",
    "approve_code_analysis",
    "remove_code_analysis",
    "delete_code_analysis_run",
    "run_search",
    "get_search_history",
    "get_search_document_content",
    "save_search_document_result",
    "mark_search_document_ingested",
    "mark_search_knowledge_ingested",
    "ingest_text",
    "run_consolidation",
    "cancel_process",
    "save_chat_session",
    "load_chat_sessions",
    "delete_chat_session",
    "check_db_ready",
    "save_file_record",
    "commit_document",
    "get_files",
    "get_document_text",
    "delete_file_record",
    "delete_file_nodes",
    "run_quick_extract",
    "search_documents",
];

pub const BACKEND_EVENTS: &[&str] = &[
    "knowledge-changed",
    "ingest-progress",
    "kb-cleanup-progress",
    "ocr-started",
    "ocr-progress",
    "code-analysis-started",
    "code-analysis-progress",
    "search-started",
    "search-progress",
    "stt-state",
    "stt-level",
    "stt-partial",
    "stt-final",
    "stt-error",
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn command_and_event_catalogs_have_no_duplicates() {
        let methods: HashSet<_> = BACKEND_METHODS.iter().copied().collect();
        let events: HashSet<_> = BACKEND_EVENTS.iter().copied().collect();
        assert_eq!(methods.len(), BACKEND_METHODS.len());
        assert_eq!(events.len(), BACKEND_EVENTS.len());
        assert!(!BACKEND_METHODS.is_empty());
    }
}
