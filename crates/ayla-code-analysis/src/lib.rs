//! Generate two complementary documents from a codebase:
//! human-facing technical documentation and graph-ready knowledge-base Markdown.

mod discovery;
mod infrastructure_parser;
mod javascript_parser;
mod markdown;
mod metadata_parser;
mod model;
mod ollama;
mod python_parser;
mod readme_parser;
mod rust_parser;
mod typescript_parser;

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub use model::{
    AnalysisControl, AnalysisMode, AnalysisProgress, Diagnostic, DiagnosticLevel, Entity,
    GenerateError, GenerateOptions, GeneratedDocuments, Relation, SourceScope,
};
pub use ollama::{
    LlmClient, OllamaClient, SemanticBatchItem, SemanticConceptRelation, SemanticResponse,
};

/// A supported input source. All variants are normalized to a directory or virtual file.
#[derive(Debug, Clone)]
pub enum CodeSource {
    Snippet {
        code: String,
        language: Option<String>,
        name: String,
    },
    Directory {
        path: PathBuf,
    },
    GitRepository {
        url: String,
        branch: Option<String>,
    },
}

/// Generates documentation. Implement this trait to plug a different semantic provider into Ayla.
#[allow(async_fn_in_trait)]
pub trait CodeDocumentationGenerator {
    async fn generate(
        &self,
        source: CodeSource,
        options: GenerateOptions,
    ) -> Result<GeneratedDocuments, GenerateError>;
}

/// Default generator using Tree-sitter for deterministic Rust facts and an optional LLM client.
pub struct Generator<C = OllamaClient> {
    llm: C,
}

impl Generator<OllamaClient> {
    pub fn new() -> Self {
        Self {
            llm: OllamaClient::default(),
        }
    }
}

impl<C> Generator<C> {
    pub fn with_llm(llm: C) -> Self {
        Self { llm }
    }
}

impl Default for Generator<OllamaClient> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: LlmClient + Send + Sync> CodeDocumentationGenerator for Generator<C> {
    async fn generate(
        &self,
        source: CodeSource,
        options: GenerateOptions,
    ) -> Result<GeneratedDocuments, GenerateError> {
        let workspace = Workspace::prepare(source, &options)?;
        let mut package = match &workspace {
            Workspace::Snippet {
                code,
                name,
                language,
            } => match language.as_deref().map(str::to_ascii_lowercase).as_deref() {
                Some("python") | Some("py") => python_parser::analyze_snippet(name, code)?,
                Some("javascript") | Some("js") | Some("mjs") | Some("cjs") => {
                    javascript_parser::analyze_snippet(name, code)?
                }
                Some("typescript") | Some("ts") | Some("tsx") => {
                    typescript_parser::analyze_snippet(name, code)?
                }
                Some("rust") | Some("rs") | None => rust_parser::analyze_snippet(name, code)?,
                _ => model::KnowledgePackage::unsupported_snippet(name, language.as_deref()),
            },
            Workspace::Directory { path, .. } => {
                discovery::analyze_directory(path, options.source_scope, None)?
            }
        };

        package.apply_semantics(&self.llm, &options).await;
        Ok(markdown::render(package, &options))
    }
}

/// Run the bounded deterministic analyzer without constructing an async runtime
/// or contacting an LLM. The control is safe to clone into a blocking worker;
/// cancellation is checked throughout discovery and fingerprinting.
pub fn generate_deterministic(
    source: CodeSource,
    mut options: GenerateOptions,
    control: &AnalysisControl,
) -> Result<GeneratedDocuments, GenerateError> {
    options.semantic_analysis = false;
    control.check()?;
    control.report(AnalysisProgress {
        stage: "preparing".into(),
        current: 0,
        total: 0,
        message: "Preparing code analysis source".into(),
    });
    let workspace = Workspace::prepare(source, &options)?;
    control.check()?;
    let package = match &workspace {
        Workspace::Snippet {
            code,
            name,
            language,
        } => {
            control.report(AnalysisProgress {
                stage: "parsing".into(),
                current: 0,
                total: 1,
                message: "Parsing source snippet".into(),
            });
            let result = match language.as_deref().map(str::to_ascii_lowercase).as_deref() {
                Some("python") | Some("py") => python_parser::analyze_snippet(name, code)?,
                Some("javascript") | Some("js") | Some("mjs") | Some("cjs") => {
                    javascript_parser::analyze_snippet(name, code)?
                }
                Some("typescript") | Some("ts") | Some("tsx") => {
                    typescript_parser::analyze_snippet(name, code)?
                }
                Some("rust") | Some("rs") | None => rust_parser::analyze_snippet(name, code)?,
                _ => model::KnowledgePackage::unsupported_snippet(name, language.as_deref()),
            };
            control.check()?;
            result
        }
        Workspace::Directory { path, .. } => {
            discovery::analyze_directory(path, options.source_scope, Some(control))?
        }
    };
    control.check()?;
    control.report(AnalysisProgress {
        stage: "rendering".into(),
        current: 1,
        total: 1,
        message: "Rendering analysis documents".into(),
    });
    let result = markdown::render(package, &options);
    control.report(AnalysisProgress {
        stage: "complete".into(),
        current: 1,
        total: 1,
        message: "Code analysis complete".into(),
    });
    Ok(result)
}

/// Writes the returned pair using `<project>.md` and `<project>.kb.md` names.
pub fn write_documents(
    output_dir: &Path,
    documents: &GeneratedDocuments,
) -> Result<(PathBuf, PathBuf), GenerateError> {
    fs::create_dir_all(output_dir)?;
    let stem = model::safe_file_stem(&documents.project_name);
    let human = output_dir.join(format!("{stem}.md"));
    let kb = output_dir.join(format!("{stem}.kb.md"));
    fs::write(&human, &documents.human_markdown)?;
    fs::write(&kb, &documents.knowledge_base_markdown)?;
    Ok((human, kb))
}

enum Workspace {
    Snippet {
        code: String,
        language: Option<String>,
        name: String,
    },
    Directory {
        path: PathBuf,
        _temporary: Option<tempfile::TempDir>,
    },
}

impl Workspace {
    fn prepare(source: CodeSource, options: &GenerateOptions) -> Result<Self, GenerateError> {
        match source {
            CodeSource::Snippet {
                code,
                language,
                name,
            } => Ok(Self::Snippet {
                code,
                language,
                name,
            }),
            CodeSource::Directory { path } => {
                if !path.is_dir() {
                    return Err(GenerateError::InvalidSource(format!(
                        "not a directory: {}",
                        path.display()
                    )));
                }
                Ok(Self::Directory {
                    path,
                    _temporary: None,
                })
            }
            CodeSource::GitRepository { url, branch } => {
                if !options.allow_remote_git {
                    return Err(GenerateError::InvalidSource(
                        "remote Git analysis is disabled; enable it explicitly for this run".into(),
                    ));
                }
                let is_https = url::Url::parse(&url)
                    .map(|parsed| parsed.scheme() == "https")
                    .unwrap_or(false);
                let is_ssh = url.starts_with("git@") || url.starts_with("ssh://");
                if !is_https && !(is_ssh && options.allow_ssh_git) {
                    return Err(GenerateError::InvalidSource(
                        "remote Git URLs must use HTTPS; SSH requires separate opt-in".into(),
                    ));
                }
                if let Some(branch) = branch.as_deref() {
                    if branch.len() > 200
                        || branch.starts_with('-')
                        || !branch.chars().all(|character| {
                            character.is_ascii_alphanumeric() || "._/-".contains(character)
                        })
                    {
                        return Err(GenerateError::InvalidSource(
                            "invalid Git branch name".into(),
                        ));
                    }
                }
                let temporary = tempfile::tempdir()?;
                let destination = temporary.path().join("repository");
                let mut command = Command::new("git");
                command.env("GIT_TERMINAL_PROMPT", "0");
                command.args(["clone", "--depth", "1", "--no-tags"]);
                if let Some(branch) = branch {
                    command.args(["--branch", &branch]);
                }
                let status = command.arg(&url).arg(&destination).status()?;
                if !status.success() {
                    return Err(GenerateError::GitClone(url));
                }
                Ok(Self::Directory {
                    path: destination,
                    _temporary: Some(temporary),
                })
            }
        }
    }
}
