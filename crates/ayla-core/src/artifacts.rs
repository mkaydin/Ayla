use crate::BackendError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactRef {
    pub id: String,
    pub kind: String,
    pub media_type: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcrArtifacts {
    pub markdown: ArtifactRef,
    pub layout: ArtifactRef,
    pub knowledge_base: ArtifactRef,
    pub structured: ArtifactRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeAnalysisArtifacts {
    pub human: ArtifactRef,
    pub knowledge_base: ArtifactRef,
    pub structured: ArtifactRef,
    #[serde(default)]
    pub diff: Option<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnalysisDiffSummary {
    pub parent_operation_id: String,
    pub added_entities: usize,
    pub removed_entities: usize,
    pub changed_entities: usize,
    pub added_relations: usize,
    pub removed_relations: usize,
    pub changed_relations: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CodeAnalysisRunRecord {
    pub schema_version: u8,
    pub operation_id: String,
    pub parent_operation_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub status: String,
    pub source_type: String,
    pub source_label: String,
    pub directory_path: Option<String>,
    pub language: Option<String>,
    pub scope: String,
    pub semantic_mode: String,
    pub project_name: String,
    pub repository_uid: String,
    pub entity_count: usize,
    pub relation_count: usize,
    pub diagnostics: Vec<ayla_code_analysis::Diagnostic>,
    pub artifacts: CodeAnalysisArtifacts,
    pub diff: Option<CodeAnalysisDiffSummary>,
    pub graph_approved: bool,
    pub vectors_indexed: bool,
    #[serde(default)]
    pub source_snapshot_stored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactContent {
    pub artifact: ArtifactRef,
    pub content: String,
    pub truncated: bool,
}

const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
const MAX_ARTIFACT_PREVIEW_BYTES: usize = 2 * 1024 * 1024;
const MAX_PRIVATE_SOURCE_BYTES: usize = 2 * 1024 * 1024;

pub fn persist_ocr_artifacts(
    operation_id: &str,
    document: &ayla_ocr::OcrDocument,
) -> Result<OcrArtifacts, BackendError> {
    validate_operation_id(operation_id)?;
    let directory = artifact_root().join("ocr").join(operation_id);
    std::fs::create_dir_all(&directory).map_err(|error| artifact_error(&directory, error))?;

    let markdown_path = directory.join("document.md");
    let layout_value = serde_json::json!({
        "schemaVersion": 1,
        "sourceName": document.source_name,
        "pagesProcessed": document.pages_processed,
        "pagesSkipped": document.pages_skipped,
        "totalPages": document.total_pages,
        "models": document.models,
        "knowledgeMode": document.knowledge_mode,
        "performance": document.performance,
        "cacheKey": document.cache_key,
        "warnings": document.warnings,
        "pages": document.page_results,
    });
    let layout_bytes = serde_json::to_vec_pretty(&layout_value).map_err(|error| {
        BackendError::new(
            "artifact_serialize",
            format!("Could not serialize OCR layout artifact: {error}"),
        )
    })?;
    let layout_path = directory.join("layout.json");

    let knowledge_base_bytes = document.knowledge.knowledge_base_markdown.as_bytes();
    let mut structured = document.knowledge.clone();
    // The Markdown has its own artifact; avoid duplicating potentially large
    // page evidence in the structured graph preview.
    structured.knowledge_base_markdown.clear();
    let structured_bytes = serde_json::to_vec_pretty(&structured).map_err(|error| {
        BackendError::new(
            "artifact_serialize",
            format!("Could not serialize OCR knowledge artifact: {error}"),
        )
    })?;
    for (label, bytes) in [
        ("OCR Markdown", document.text.as_bytes()),
        ("OCR layout", layout_bytes.as_slice()),
        ("OCR knowledge-base Markdown", knowledge_base_bytes),
        ("OCR structured knowledge", structured_bytes.as_slice()),
    ] {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(BackendError::new(
                "artifact_too_large",
                format!("The {label} artifact exceeds the 64 MiB safety limit"),
            ));
        }
    }
    atomic_write(&markdown_path, document.text.as_bytes())?;
    atomic_write(&layout_path, &layout_bytes)?;
    atomic_write(&directory.join("knowledge-base.md"), knowledge_base_bytes)?;
    atomic_write(&directory.join("knowledge.json"), &structured_bytes)?;

    Ok(OcrArtifacts {
        markdown: ArtifactRef {
            id: format!("ocr:{operation_id}:markdown"),
            kind: "ocr_markdown".into(),
            media_type: "text/markdown".into(),
            bytes: document.text.len(),
        },
        layout: ArtifactRef {
            id: format!("ocr:{operation_id}:layout"),
            kind: "ocr_layout".into(),
            media_type: "application/json".into(),
            bytes: layout_bytes.len(),
        },
        knowledge_base: ocr_artifact_ref(
            operation_id,
            "knowledge_base",
            "text/markdown",
            knowledge_base_bytes.len(),
        ),
        structured: ocr_artifact_ref(
            operation_id,
            "structured",
            "application/json",
            structured_bytes.len(),
        ),
    })
}

pub fn persist_code_analysis_artifacts(
    operation_id: &str,
    documents: &ayla_code_analysis::GeneratedDocuments,
) -> Result<CodeAnalysisArtifacts, BackendError> {
    validate_operation_id(operation_id)?;
    let directory = artifact_root().join("code-analysis").join(operation_id);
    std::fs::create_dir_all(&directory).map_err(|error| artifact_error(&directory, error))?;

    // Source excerpts are parser inputs, not durable knowledge. Removing them
    // keeps the artifact bounded and prevents copying entire source files into
    // Ayla's data directory.
    let mut sanitized = documents.clone();
    for entity in &mut sanitized.entities {
        entity.source_excerpt = None;
    }
    let structured_bytes = serde_json::to_vec_pretty(&sanitized).map_err(|error| {
        BackendError::new(
            "artifact_serialize",
            format!("Could not serialize code-analysis artifact: {error}"),
        )
    })?;

    let human_bytes = documents.human_markdown.as_bytes();
    let knowledge_base_bytes = documents.knowledge_base_markdown.as_bytes();
    for (label, bytes) in [
        ("human documentation", human_bytes),
        ("knowledge-base documentation", knowledge_base_bytes),
        ("structured analysis", structured_bytes.as_slice()),
    ] {
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(BackendError::new(
                "artifact_too_large",
                format!("The {label} artifact exceeds the 64 MiB safety limit"),
            ));
        }
    }

    atomic_write(&directory.join("documentation.md"), human_bytes)?;
    atomic_write(&directory.join("knowledge-base.md"), knowledge_base_bytes)?;
    atomic_write(&directory.join("result.json"), &structured_bytes)?;

    Ok(CodeAnalysisArtifacts {
        human: code_artifact_ref(operation_id, "human", human_bytes.len()),
        knowledge_base: code_artifact_ref(
            operation_id,
            "knowledge_base",
            knowledge_base_bytes.len(),
        ),
        structured: code_artifact_ref(operation_id, "structured", structured_bytes.len()),
        diff: None,
    })
}

pub fn persist_code_analysis_diff(
    operation_id: &str,
    parent_operation_id: &str,
    current: &ayla_code_analysis::GeneratedDocuments,
    previous: &ayla_code_analysis::GeneratedDocuments,
) -> Result<(CodeAnalysisDiffSummary, ArtifactRef), BackendError> {
    validate_operation_id(operation_id)?;
    validate_operation_id(parent_operation_id)?;
    let current_entities = durable_entities(current);
    let previous_entities = durable_entities(previous);
    let current_relations = durable_relations(current);
    let previous_relations = durable_relations(previous);
    let current_entity_ids = current_entities.keys().cloned().collect::<BTreeSet<_>>();
    let previous_entity_ids = previous_entities.keys().cloned().collect::<BTreeSet<_>>();
    let current_relation_ids = current_relations.keys().cloned().collect::<BTreeSet<_>>();
    let previous_relation_ids = previous_relations.keys().cloned().collect::<BTreeSet<_>>();
    let added_entity_uids = current_entity_ids
        .difference(&previous_entity_ids)
        .cloned()
        .collect::<Vec<_>>();
    let removed_entity_uids = previous_entity_ids
        .difference(&current_entity_ids)
        .cloned()
        .collect::<Vec<_>>();
    let changed_entity_uids = current_entity_ids
        .intersection(&previous_entity_ids)
        .filter(|uid| current_entities.get(*uid) != previous_entities.get(*uid))
        .cloned()
        .collect::<Vec<_>>();
    let added_relation_uids = current_relation_ids
        .difference(&previous_relation_ids)
        .cloned()
        .collect::<Vec<_>>();
    let removed_relation_uids = previous_relation_ids
        .difference(&current_relation_ids)
        .cloned()
        .collect::<Vec<_>>();
    let changed_relation_uids = current_relation_ids
        .intersection(&previous_relation_ids)
        .filter(|uid| current_relations.get(*uid) != previous_relations.get(*uid))
        .cloned()
        .collect::<Vec<_>>();
    let summary = CodeAnalysisDiffSummary {
        parent_operation_id: parent_operation_id.into(),
        added_entities: added_entity_uids.len(),
        removed_entities: removed_entity_uids.len(),
        changed_entities: changed_entity_uids.len(),
        added_relations: added_relation_uids.len(),
        removed_relations: removed_relation_uids.len(),
        changed_relations: changed_relation_uids.len(),
    };
    let value = serde_json::json!({
        "schemaVersion": 1,
        "summary": summary,
        "addedEntityUids": added_entity_uids,
        "removedEntityUids": removed_entity_uids,
        "changedEntityUids": changed_entity_uids,
        "addedRelationUids": added_relation_uids,
        "removedRelationUids": removed_relation_uids,
        "changedRelationUids": changed_relation_uids,
    });
    let bytes = serde_json::to_vec_pretty(&value).map_err(|error| {
        BackendError::new(
            "artifact_serialize",
            format!("Could not serialize code-analysis diff: {error}"),
        )
    })?;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(BackendError::new(
            "artifact_too_large",
            "Code-analysis diff exceeds the 64 MiB safety limit",
        ));
    }
    let path = artifact_root()
        .join("code-analysis")
        .join(operation_id)
        .join("diff.json");
    atomic_write(&path, &bytes)?;
    Ok((
        summary,
        ArtifactRef {
            id: format!("code-analysis:{operation_id}:diff"),
            kind: "code_analysis_diff".into(),
            media_type: "application/json".into(),
            bytes: bytes.len(),
        },
    ))
}

pub fn persist_code_analysis_run(run: &CodeAnalysisRunRecord) -> Result<(), BackendError> {
    validate_operation_id(&run.operation_id)?;
    let bytes = serde_json::to_vec_pretty(run).map_err(|error| {
        BackendError::new(
            "artifact_serialize",
            format!("Could not serialize code-analysis run: {error}"),
        )
    })?;
    if bytes.len() > MAX_ARTIFACT_PREVIEW_BYTES {
        return Err(BackendError::new(
            "artifact_too_large",
            "Code-analysis run metadata exceeds the 2 MiB safety limit",
        ));
    }
    let path = artifact_root()
        .join("code-analysis")
        .join(&run.operation_id)
        .join("run.json");
    atomic_write(&path, &bytes)
}

/// Persist source needed to reproduce a pasted-snippet run. This file is
/// deliberately absent from `CodeAnalysisArtifacts` and `resolve_artifact`, so
/// renderers cannot request it through the general preview API.
pub fn persist_code_analysis_private_source(
    operation_id: &str,
    source: &str,
) -> Result<(), BackendError> {
    validate_operation_id(operation_id)?;
    if source.is_empty() || source.len() > MAX_PRIVATE_SOURCE_BYTES {
        return Err(BackendError::new(
            "private_source_invalid",
            "Private snippet source must contain between 1 byte and 2 MiB",
        ));
    }
    let path = private_source_path(operation_id);
    atomic_write_private(&path, source.as_bytes())
}

pub fn load_code_analysis_private_source(operation_id: &str) -> Result<String, BackendError> {
    validate_operation_id(operation_id)?;
    let path = private_source_path(operation_id);
    let metadata =
        std::fs::symlink_metadata(&path).map_err(|error| read_artifact_error(&path, error))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(BackendError::new(
            "private_source_rejected",
            "Private snippet source is not a regular file",
        ));
    }
    if metadata.len() == 0 || metadata.len() > MAX_PRIVATE_SOURCE_BYTES as u64 {
        return Err(BackendError::new(
            "private_source_invalid",
            "Private snippet source must contain between 1 byte and 2 MiB",
        ));
    }
    std::fs::read_to_string(&path).map_err(|error| read_artifact_error(&path, error))
}

pub fn load_code_analysis_run(operation_id: &str) -> Result<CodeAnalysisRunRecord, BackendError> {
    validate_operation_id(operation_id)?;
    let path = artifact_root()
        .join("code-analysis")
        .join(operation_id)
        .join("run.json");
    let metadata = std::fs::metadata(&path).map_err(|error| read_artifact_error(&path, error))?;
    if metadata.len() > MAX_ARTIFACT_PREVIEW_BYTES as u64 {
        return Err(BackendError::new(
            "artifact_too_large",
            "Code-analysis run metadata exceeds the 2 MiB safety limit",
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| read_artifact_error(&path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        BackendError::new(
            "artifact_invalid",
            format!("Code-analysis run metadata is invalid: {error}"),
        )
    })
}

pub fn update_code_analysis_run(
    operation_id: &str,
    update: impl FnOnce(&mut CodeAnalysisRunRecord),
) -> Result<CodeAnalysisRunRecord, BackendError> {
    let mut run = load_code_analysis_run(operation_id)?;
    update(&mut run);
    run.updated_at = chrono::Utc::now().to_rfc3339();
    persist_code_analysis_run(&run)?;
    Ok(run)
}

pub fn list_code_analysis_runs() -> Result<Vec<CodeAnalysisRunRecord>, BackendError> {
    let root = artifact_root().join("code-analysis");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut runs = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter_map(|operation_id| load_code_analysis_run(&operation_id).ok())
        .collect::<Vec<_>>();
    runs.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    runs.truncate(200);
    Ok(runs)
}

pub fn delete_code_analysis_artifacts(operation_id: &str) -> Result<(), BackendError> {
    validate_operation_id(operation_id)?;
    let directory = artifact_root().join("code-analysis").join(operation_id);
    let metadata = std::fs::symlink_metadata(&directory)
        .map_err(|error| read_artifact_error(&directory, error))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(BackendError::new(
            "artifact_delete_rejected",
            "Code-analysis artifact target is not a regular directory",
        ));
    }
    std::fs::remove_dir_all(&directory).map_err(|error| {
        BackendError::new(
            "artifact_delete",
            format!(
                "Could not delete code-analysis artifacts at {}: {error}",
                directory.display()
            ),
        )
    })
}

pub fn read_artifact_content(artifact_id: &str) -> Result<ArtifactContent, BackendError> {
    let (artifact, path) = resolve_artifact(artifact_id)?;
    let file = std::fs::File::open(&path).map_err(|error| read_artifact_error(&path, error))?;
    let mut preview = Vec::with_capacity(MAX_ARTIFACT_PREVIEW_BYTES.min(64 * 1024));
    file.take(MAX_ARTIFACT_PREVIEW_BYTES as u64 + 1)
        .read_to_end(&mut preview)
        .map_err(|error| read_artifact_error(&path, error))?;
    let truncated = preview.len() > MAX_ARTIFACT_PREVIEW_BYTES;
    preview.truncate(MAX_ARTIFACT_PREVIEW_BYTES);
    let content = String::from_utf8_lossy(&preview).into_owned();
    Ok(ArtifactContent {
        artifact,
        content,
        truncated,
    })
}

pub fn load_code_analysis_result(
    operation_id: &str,
) -> Result<ayla_code_analysis::GeneratedDocuments, BackendError> {
    validate_operation_id(operation_id)?;
    let path = artifact_root()
        .join("code-analysis")
        .join(operation_id)
        .join("result.json");
    let metadata = std::fs::metadata(&path).map_err(|error| read_artifact_error(&path, error))?;
    if metadata.len() > MAX_ARTIFACT_BYTES as u64 {
        return Err(BackendError::new(
            "artifact_too_large",
            "Structured code-analysis artifact exceeds the 64 MiB safety limit",
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| read_artifact_error(&path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        BackendError::new(
            "artifact_invalid",
            format!("Structured code-analysis artifact is invalid: {error}"),
        )
    })
}

pub fn load_ocr_knowledge_result(
    operation_id: &str,
) -> Result<ayla_ocr::OcrKnowledgePackage, BackendError> {
    validate_operation_id(operation_id)?;
    let directory = artifact_root().join("ocr").join(operation_id);
    let structured_path = directory.join("knowledge.json");
    let metadata = std::fs::metadata(&structured_path)
        .map_err(|error| read_artifact_error(&structured_path, error))?;
    if metadata.len() > MAX_ARTIFACT_BYTES as u64 {
        return Err(BackendError::new(
            "artifact_too_large",
            "Structured OCR knowledge artifact exceeds the 64 MiB safety limit",
        ));
    }
    let bytes = std::fs::read(&structured_path)
        .map_err(|error| read_artifact_error(&structured_path, error))?;
    let mut package: ayla_ocr::OcrKnowledgePackage =
        serde_json::from_slice(&bytes).map_err(|error| {
            BackendError::new(
                "artifact_invalid",
                format!("Structured OCR knowledge artifact is invalid: {error}"),
            )
        })?;
    if package.schema_version != 1
        || package.source_name.trim().is_empty()
        || package.document_uid.trim().is_empty()
    {
        return Err(BackendError::new(
            "artifact_invalid",
            "Structured OCR knowledge artifact has invalid identity metadata",
        ));
    }
    let markdown_path = directory.join("knowledge-base.md");
    let markdown_metadata = std::fs::metadata(&markdown_path)
        .map_err(|error| read_artifact_error(&markdown_path, error))?;
    if markdown_metadata.len() > MAX_ARTIFACT_BYTES as u64 {
        return Err(BackendError::new(
            "artifact_too_large",
            "OCR knowledge-base Markdown exceeds the 64 MiB safety limit",
        ));
    }
    package.knowledge_base_markdown = std::fs::read_to_string(&markdown_path)
        .map_err(|error| read_artifact_error(&markdown_path, error))?;
    Ok(package)
}

fn ocr_artifact_ref(operation_id: &str, kind: &str, media_type: &str, bytes: usize) -> ArtifactRef {
    ArtifactRef {
        id: format!("ocr:{operation_id}:{kind}"),
        kind: format!("ocr_{kind}"),
        media_type: media_type.into(),
        bytes,
    }
}

fn code_artifact_ref(operation_id: &str, kind: &str, bytes: usize) -> ArtifactRef {
    ArtifactRef {
        id: format!("code-analysis:{operation_id}:{kind}"),
        kind: format!("code_analysis_{kind}"),
        media_type: if kind == "structured" {
            "application/json".into()
        } else {
            "text/markdown".into()
        },
        bytes,
    }
}

fn resolve_artifact(artifact_id: &str) -> Result<(ArtifactRef, PathBuf), BackendError> {
    let parts = artifact_id.split(':').collect::<Vec<_>>();
    if parts.len() != 3 || !matches!(parts[0], "code-analysis" | "ocr") {
        return Err(BackendError::new(
            "invalid_artifact_id",
            "Unsupported artifact ID",
        ));
    }
    validate_operation_id(parts[1])?;
    let (directory, filename, kind, media_type) = match (parts[0], parts[2]) {
        ("code-analysis", "human") => (
            "code-analysis",
            "documentation.md",
            "code_analysis_human",
            "text/markdown",
        ),
        ("code-analysis", "knowledge_base") => (
            "code-analysis",
            "knowledge-base.md",
            "code_analysis_knowledge_base",
            "text/markdown",
        ),
        ("code-analysis", "structured") => (
            "code-analysis",
            "result.json",
            "code_analysis_structured",
            "application/json",
        ),
        ("code-analysis", "diff") => (
            "code-analysis",
            "diff.json",
            "code_analysis_diff",
            "application/json",
        ),
        ("ocr", "markdown") => ("ocr", "document.md", "ocr_markdown", "text/markdown"),
        ("ocr", "layout") => ("ocr", "layout.json", "ocr_layout", "application/json"),
        ("ocr", "knowledge_base") => (
            "ocr",
            "knowledge-base.md",
            "ocr_knowledge_base",
            "text/markdown",
        ),
        ("ocr", "structured") => (
            "ocr",
            "knowledge.json",
            "ocr_structured",
            "application/json",
        ),
        _ => {
            return Err(BackendError::new(
                "invalid_artifact_id",
                "Unknown artifact kind",
            ))
        }
    };
    let path = artifact_root()
        .join(directory)
        .join(parts[1])
        .join(filename);
    let bytes = std::fs::metadata(&path)
        .map_err(|error| read_artifact_error(&path, error))?
        .len() as usize;
    Ok((
        ArtifactRef {
            id: artifact_id.into(),
            kind: kind.into(),
            media_type: media_type.into(),
            bytes,
        },
        path,
    ))
}

fn durable_entities(
    documents: &ayla_code_analysis::GeneratedDocuments,
) -> BTreeMap<String, serde_json::Value> {
    documents
        .entities
        .iter()
        .cloned()
        .map(|mut entity| {
            entity.source_excerpt = None;
            let uid = logical_code_uid(&entity.uid);
            entity.uid = uid.clone();
            (uid, serde_json::to_value(entity).unwrap_or_default())
        })
        .collect()
}

fn durable_relations(
    documents: &ayla_code_analysis::GeneratedDocuments,
) -> BTreeMap<String, serde_json::Value> {
    documents
        .relations
        .iter()
        .cloned()
        .map(|mut relation| {
            relation.source = logical_code_uid(&relation.source);
            relation.target = logical_code_uid(&relation.target);
            let uid = format!(
                "{}|{}|{}|{}",
                relation.source,
                relation.relation,
                relation.target,
                relation.source_location.as_deref().unwrap_or_default()
            );
            (uid, serde_json::to_value(relation).unwrap_or_default())
        })
        .collect()
}

fn logical_code_uid(uid: &str) -> String {
    let Some(rest) = uid.strip_prefix("repo:") else {
        return uid.to_string();
    };
    match rest.split_once(':') {
        Some((_, suffix)) => format!("repo:*:{suffix}"),
        None => "repo:*".into(),
    }
}

fn validate_operation_id(operation_id: &str) -> Result<(), BackendError> {
    if operation_id.is_empty()
        || operation_id.len() > 64
        || !operation_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err(BackendError::new(
            "invalid_artifact_operation",
            "Artifact operation ID contains unsupported characters",
        ));
    }
    Ok(())
}

fn artifact_root() -> PathBuf {
    std::env::var_os("AYLA_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join("ayla-data")
        })
        .join("artifacts")
}

fn private_source_path(operation_id: &str) -> PathBuf {
    artifact_root()
        .join("code-analysis")
        .join(operation_id)
        .join("source.snippet")
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), BackendError> {
    let temporary = path.with_extension(format!(
        "{}.part",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("artifact")
    ));
    std::fs::write(&temporary, contents).map_err(|error| artifact_error(&temporary, error))?;
    std::fs::rename(&temporary, path).map_err(|error| artifact_error(path, error))
}

fn atomic_write_private(path: &Path, contents: &[u8]) -> Result<(), BackendError> {
    let temporary = path.with_extension("snippet.part");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| artifact_error(&temporary, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|error| artifact_error(&temporary, error))?;
    }
    file.write_all(contents)
        .map_err(|error| artifact_error(&temporary, error))?;
    file.sync_all()
        .map_err(|error| artifact_error(&temporary, error))?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|error| artifact_error(path, error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| artifact_error(path, error))?;
    }
    Ok(())
}

fn artifact_error(path: &Path, error: std::io::Error) -> BackendError {
    BackendError::new(
        "artifact_io",
        format!("Could not write artifact {}: {error}", path.display()),
    )
}

fn read_artifact_error(path: &Path, error: std::io::Error) -> BackendError {
    BackendError::new(
        "artifact_read",
        format!("Could not read artifact {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayla_ocr::{LayoutSource, OcrDocument, OcrKnowledgePackage, OcrModels, OcrPageResult};

    #[test]
    fn rejects_path_like_operation_ids() {
        let document = fixture_document();
        let error = persist_ocr_artifacts("../escape", &document).unwrap_err();
        assert_eq!(error.code, "invalid_artifact_operation");
    }

    #[test]
    fn revision_comparison_ignores_repository_fingerprint_prefix() {
        assert_eq!(logical_code_uid("repo:abc123"), "repo:*");
        assert_eq!(
            logical_code_uid("repo:abc123:method:src/lib.rs::Parser::parse"),
            "repo:*:method:src/lib.rs::Parser::parse"
        );
        assert_eq!(logical_code_uid("external:serde"), "external:serde");
    }

    fn fixture_document() -> OcrDocument {
        OcrDocument {
            text: "# Fixture\n".into(),
            source_name: "fixture.pdf".into(),
            pages_processed: 1,
            pages_skipped: 0,
            total_pages: 1,
            page_results: vec![OcrPageResult {
                page: 1,
                chars: 9,
                columns: 1,
                headings: vec!["Fixture".into()],
                figures: vec![],
                layout_source: LayoutSource::PdfText,
                warnings: vec![],
                cached: false,
            }],
            models: OcrModels {
                revision: None,
                ocr: "glm-ocr:latest".into(),
                assembly: None,
            },
            warnings: vec![],
            knowledge: OcrKnowledgePackage {
                schema_version: 1,
                source_name: "fixture.pdf".into(),
                document_uid: "ocr-document:fixture".into(),
                knowledge_base_markdown: "# Knowledge Base Document: Fixture\n".into(),
                entities: vec![],
                relations: vec![],
                warnings: vec![],
            },
            knowledge_mode: ayla_ocr::OcrKnowledgeMode::Deterministic,
            performance: ayla_ocr::OcrPerformanceMetrics::default(),
            cache_key: None,
        }
    }
}
