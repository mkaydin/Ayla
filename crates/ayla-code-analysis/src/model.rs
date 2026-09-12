use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub enum AnalysisMode {
    Fast,
    #[default]
    Standard,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceScope {
    All,
    #[default]
    DocumentsOnly,
}

#[derive(Debug, Clone)]
pub struct GenerateOptions {
    pub model: String,
    pub ollama_base_url: Url,
    pub analysis_mode: AnalysisMode,
    pub output_name: Option<String>,
    pub semantic_analysis: bool,
    /// Maximum number of code symbols interpreted by the LLM. `None` means all symbols.
    pub semantic_symbol_limit: Option<usize>,
    /// Maximum number of documentation nodes interpreted by the LLM. `None` means all documents.
    pub semantic_document_limit: Option<usize>,
    pub source_scope: SourceScope,
    /// Remote repositories are disabled in the desktop application unless the
    /// user explicitly enables them.
    pub allow_remote_git: bool,
    /// SSH Git URLs can invoke credential agents and are separately gated.
    pub allow_ssh_git: bool,
}

impl Default for GenerateOptions {
    fn default() -> Self {
        Self {
            model: "nemotron-3-super:cloud".into(),
            ollama_base_url: Url::parse("http://localhost:11434/").unwrap(),
            analysis_mode: AnalysisMode::Standard,
            output_name: None,
            semantic_analysis: false,
            semantic_symbol_limit: Some(36),
            semantic_document_limit: Some(6),
            source_scope: SourceScope::DocumentsOnly,
            allow_remote_git: false,
            allow_ssh_git: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entity {
    pub uid: String,
    pub label: String,
    pub name: String,
    pub description: Option<String>,
    pub source_location: Option<String>,
    pub origin: String,
    pub confidence: f32,
    pub embedding_text: Option<String>,
    /// Structured facts that downstream graph and vector stores can ingest without parsing prose.
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
    /// Original source for semantic analysis. It is deliberately not rendered into KB metadata.
    pub source_excerpt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Relation {
    pub source: String,
    pub relation: String,
    pub target: String,
    pub origin: String,
    pub confidence: f32,
    pub source_location: Option<String>,
    /// Edge-specific facts, such as a dependency version constraint or declaration scope.
    #[serde(default)]
    pub properties: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GeneratedDocuments {
    pub project_name: String,
    pub repository_uid: String,
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
    pub human_markdown: String,
    pub knowledge_base_markdown: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AnalysisProgress {
    pub stage: String,
    pub current: usize,
    pub total: usize,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct AnalysisControl {
    cancelled: Arc<AtomicBool>,
    progress: Option<Arc<dyn Fn(AnalysisProgress) + Send + Sync>>,
}

impl AnalysisControl {
    pub fn with_progress(progress: impl Fn(AnalysisProgress) + Send + Sync + 'static) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            progress: Some(Arc::new(progress)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn check(&self) -> Result<(), GenerateError> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(GenerateError::Cancelled)
        } else {
            Ok(())
        }
    }

    pub(crate) fn report(&self, update: AnalysisProgress) {
        if let Some(progress) = &self.progress {
            progress(update);
        }
    }
}

#[derive(Debug, Error)]
pub enum GenerateError {
    #[error("invalid source: {0}")]
    InvalidSource(String),
    #[error("git clone failed for {0}")]
    GitClone(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Tree-sitter error: {0}")]
    Parse(String),
    #[error("analysis cancelled")]
    Cancelled,
    #[error("analysis limit exceeded: {0}")]
    Limit(String),
}

#[derive(Debug, Clone)]
pub(crate) struct KnowledgePackage {
    pub project_name: String,
    pub repository_uid: String,
    pub entities: Vec<Entity>,
    pub relations: Vec<Relation>,
    pub diagnostics: Vec<Diagnostic>,
}

impl KnowledgePackage {
    pub fn new(project_name: String, fingerprint: &str) -> Self {
        let repository_uid = format!("repo:{fingerprint}");
        let repository = Entity {
            uid: repository_uid.clone(),
            label: "Repository".into(),
            name: project_name.clone(),
            description: None,
            source_location: None,
            origin: "DISCOVERY".into(),
            confidence: 1.0,
            embedding_text: None,
            properties: BTreeMap::new(),
            source_excerpt: None,
        };
        Self {
            project_name,
            repository_uid,
            entities: vec![repository],
            relations: vec![],
            diagnostics: vec![],
        }
    }
    pub fn unsupported_snippet(name: &str, language: Option<&str>) -> Self {
        let mut result = Self::new(name.to_owned(), &short_hash(name));
        result.diagnostics.push(Diagnostic {
            level: DiagnosticLevel::Warning,
            message: format!(
                "No structural analyzer for {}",
                language.unwrap_or("unknown language")
            ),
        });
        result
    }
    pub fn add_entity(&mut self, entity: Entity) {
        self.entities.push(entity);
    }
    pub fn add_relation(&mut self, relation: Relation) {
        self.relations.push(relation);
    }
    pub fn deduplicate(&mut self) {
        let mut entity_ids = BTreeSet::new();
        self.entities.retain(|e| entity_ids.insert(e.uid.clone()));
        let mut relation_ids = BTreeSet::new();
        self.relations.retain(|r| {
            relation_ids.insert(format!(
                "{}|{}|{}|{}",
                r.source,
                r.relation,
                r.target,
                r.source_location.as_deref().unwrap_or("")
            ))
        });
    }
}

pub(crate) fn short_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(value.as_bytes()))[..12].to_string()
}
pub fn safe_file_stem(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

pub(crate) fn code_embedding_text(label: &str, name: &str, file: &str, source: &str) -> String {
    let signature = source
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    let signature = signature.chars().take(500).collect::<String>();
    if signature.is_empty() {
        format!("{label} {name} defined in {file}.")
    } else {
        format!("{label} {name} defined in {file}. Signature or declaration: {signature}")
    }
}
