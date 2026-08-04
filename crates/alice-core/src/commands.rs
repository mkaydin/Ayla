/// Protocol command catalog. Phase 2 uses this exact allowlist during the
/// Electron/Rust handshake.
pub const BACKEND_METHODS: &[&str] = &[
    "generate_tts",
    "stop_tts",
    "enrich_prompt",
    "process_turn",
    "process_message",
    "stream_chat",
    "get_kb_data",
    "analyze_and_merge_kb",
    "run_ocr",
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
    "get_files",
    "get_document_text",
    "delete_file_record",
    "delete_file_nodes",
    "run_quick_extract",
    "search_documents",
];

pub const BACKEND_EVENTS: &[&str] = &[
    "ingest-progress",
    "kb-cleanup-progress",
    "ocr-started",
    "search-started",
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
        assert_eq!(BACKEND_METHODS.len(), 29);
    }
}
