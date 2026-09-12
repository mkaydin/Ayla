use ayla_code_analysis::{
    generate_deterministic, AnalysisControl, CodeDocumentationGenerator, CodeSource, Entity,
    GenerateError, GenerateOptions, Generator, LlmClient, SemanticConceptRelation,
    SemanticResponse, SourceScope,
};

#[test]
fn deterministic_analysis_honors_preflight_cancellation() {
    let control = AnalysisControl::default();
    control.cancel();
    let result = generate_deterministic(
        CodeSource::Snippet {
            name: "cancelled.rs".into(),
            language: Some("rust".into()),
            code: "struct NeverParsed;".into(),
        },
        GenerateOptions::default(),
        &control,
    );
    assert!(matches!(result, Err(GenerateError::Cancelled)));
}

#[tokio::test]
async fn rust_snippet_generates_both_document_forms_without_llm() {
    let source = CodeSource::Snippet {
        name: "example".into(),
        language: Some("rust".into()),
        code: "use std::fmt;\nstruct Parser;\nimpl Parser { fn parse(&self) {} }\n".into(),
    };
    let documents = Generator::new()
        .generate(
            source,
            GenerateOptions {
                semantic_analysis: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(documents.human_markdown.contains("# example"));
    assert!(documents.human_markdown.contains("Parser::parse"));
    assert!(documents
        .knowledge_base_markdown
        .contains("Graph Ontology Definition"));
    assert!(documents.knowledge_base_markdown.contains("`DEFINES`"));
    assert!(documents.knowledge_base_markdown.contains("Struct"));
    assert!(documents.repository_uid.starts_with("repo:"));
    assert!(documents
        .entities
        .iter()
        .any(|entity| entity.name == "Parser"));
    assert!(documents
        .relations
        .iter()
        .any(|relation| relation.relation == "DEFINES"));
}

#[tokio::test]
async fn directory_fingerprint_is_stable_across_absolute_paths() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    for directory in [first.path(), second.path()] {
        std::fs::write(directory.join("README.md"), "# Same project\n").unwrap();
        std::fs::write(directory.join("lib.rs"), "pub struct Stable;\n").unwrap();
    }
    let options = GenerateOptions {
        semantic_analysis: false,
        source_scope: SourceScope::All,
        ..Default::default()
    };
    let first_result = Generator::new()
        .generate(
            CodeSource::Directory {
                path: first.path().to_path_buf(),
            },
            options.clone(),
        )
        .await
        .unwrap();
    let second_result = Generator::new()
        .generate(
            CodeSource::Directory {
                path: second.path().to_path_buf(),
            },
            options,
        )
        .await
        .unwrap();
    assert_eq!(first_result.repository_uid, second_result.repository_uid);
}

#[tokio::test]
async fn remote_git_is_disabled_without_explicit_consent() {
    let error = Generator::new()
        .generate(
            CodeSource::GitRepository {
                url: "https://example.com/project.git".into(),
                branch: None,
            },
            GenerateOptions {
                semantic_analysis: false,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("remote Git analysis is disabled"));
}

#[tokio::test]
async fn python_snippet_extracts_classes_methods_and_imports() {
    let source = CodeSource::Snippet {
        name: "python-example".into(),
        language: Some("python".into()),
        code: "from pathlib import Path\nclass Loader:\n    def load(self):\n        return Path('.')\n".into(),
    };
    let documents = Generator::new()
        .generate(
            source,
            GenerateOptions {
                semantic_analysis: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(documents.human_markdown.contains("Loader"));
    assert!(documents.human_markdown.contains("Loader::load"));
    assert!(documents
        .knowledge_base_markdown
        .contains("Python import discovered"));
}

#[tokio::test]
async fn readme_information_populates_human_and_kb_outputs_before_llm() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("README.md"),
        "# Atlas\n\nAtlas coordinates repository knowledge ingestion.\n\n## Usage\n\nRun the service.",
    )
    .unwrap();
    std::fs::write(directory.path().join("main.py"), "def run():\n    pass\n").unwrap();
    let documents = Generator::new()
        .generate(
            CodeSource::Directory {
                path: directory.path().to_path_buf(),
            },
            GenerateOptions {
                semantic_analysis: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(documents.human_markdown.contains("Key Project Documents"));
    assert!(documents
        .human_markdown
        .contains("Atlas coordinates repository knowledge ingestion."));
    assert!(documents
        .knowledge_base_markdown
        .contains("### Documentation (`Documentation`)"));
    assert!(documents.knowledge_base_markdown.contains("origin: README"));
    assert!(documents
        .knowledge_base_markdown
        .contains("`DOCUMENTED_BY`"));
}

#[tokio::test]
async fn document_scope_materializes_manifests_dependencies_and_contains_edges() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("README.md"),
        "# Atlas\n\nAtlas API.\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("requirements.txt"),
        "fastapi==0.115.2\nuvicorn[standard]>=0.30,<1\n",
    )
    .unwrap();
    let documents = Generator::new()
        .generate(
            CodeSource::Directory {
                path: directory.path().to_path_buf(),
            },
            GenerateOptions {
                semantic_analysis: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let kb = documents.knowledge_base_markdown;
    assert!(kb.contains("package:pypi:fastapi"));
    assert!(kb.contains("package:pypi:uvicorn"));
    assert!(kb.contains("`DECLARES_DEPENDENCY`"));
    assert!(kb.contains("versionConstraint"));
    assert!(kb.contains("==0.115.2"));
    assert!(kb.contains("`CONTAINS`"));
    assert!(kb.contains(":documentation:README.md"));
}

#[tokio::test]
async fn document_scope_extracts_safe_config_license_and_external_ids() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("T1547.001-response-playbook.md"),
        "# Response playbook\n\nTactic: Persistence\nTool: Registry Cleaner\nMitigate Registry Run Keys.\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join(".env.example"),
        "DATABASE_URL=postgres://placeholder\nAPI_TOKEN=do-not-store\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("LICENSE"),
        "MIT License\n\nCopyright (c) 2026 Atlas Authors\n\nPermission is hereby granted, free of charge, to any person obtaining a copy",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("compose.yml"),
        "services:\n  api:\n    image: example/api:1.2\n    environment:\n      API_TOKEN: placeholder\n",
    )
    .unwrap();
    let documents = Generator::new()
        .generate(
            CodeSource::Directory {
                path: directory.path().to_path_buf(),
            },
            GenerateOptions {
                semantic_analysis: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let kb = documents.knowledge_base_markdown;
    assert!(kb.contains("external:mitre-attack:T1547.001"));
    assert!(kb.contains("`ADDRESSES_TECHNIQUE`"));
    assert!(kb.contains("`PART_OF_TACTIC`"));
    assert!(kb.contains("`USES_TECHNIQUE`"));
    assert!(kb.contains("Persistence"));
    assert!(kb.contains("Registry Cleaner"));
    assert!(kb.contains("DATABASE_URL"));
    assert!(kb.contains("API_TOKEN"));
    assert!(!kb.contains("postgres://placeholder"));
    assert!(!kb.contains("do-not-store"));
    assert!(kb.contains("licenseType"));
    assert!(kb.contains("MIT"));
    assert!(kb.contains("SystemOverview"));
    assert!(kb.contains("`USES_CONFIG_KEY`"));
    assert!(kb.contains("example/api:1.2"));
    assert!(!kb.contains("API_TOKEN: placeholder"));
}

struct GraphConceptLlm;

impl LlmClient for GraphConceptLlm {
    async fn describe(
        &self,
        _model: &str,
        _base_url: &url::Url,
        _entity: &Entity,
    ) -> Result<SemanticResponse, String> {
        Ok(SemanticResponse {
            purpose: "Describes conflict resolution through moderation.".into(),
            responsibilities: vec![],
            concepts: vec!["conflict resolution".into(), "moderation".into()],
            relationships: vec![SemanticConceptRelation {
                source: "Moderation".into(),
                relation: "SUPPORTS".into(),
                target: "ConflictResolution".into(),
                confidence: 0.75,
            }],
        })
    }
}

#[tokio::test]
async fn semantic_concepts_are_canonical_and_related_across_documents() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("system");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(
        project.join("README.md"),
        "# System\n\nModeration resolves conflicts.",
    )
    .unwrap();
    std::fs::write(
        project.join("guide.md"),
        "# Guide\n\nUse moderation for conflict resolution.",
    )
    .unwrap();
    let documents = Generator::with_llm(GraphConceptLlm)
        .generate(
            CodeSource::Directory { path: project },
            GenerateOptions {
                semantic_analysis: true,
                semantic_document_limit: None,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let kb = documents.knowledge_base_markdown;
    assert_eq!(
        kb.matches("uid: `concept:system.domain.moderation`")
            .count(),
        1
    );
    assert!(kb.contains("\"sourceLocations\":[\"README.md\",\"guide.md\"]"));
    assert!(kb.contains("\"aliases\":[\"conflict resolution\",\"ConflictResolution\"]"));
    assert!(kb.contains("| `concept:system.domain.moderation` | `SUPPORTS` | `concept:system.domain.conflictresolution` |"));
}

#[tokio::test]
async fn documents_only_scope_excludes_code_and_includes_project_documents() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("README.md"),
        "# Project\n\nRoot overview.",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("guide.md"),
        "# Guide\n\nOperational guidance.",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("main.py"),
        "def hidden_from_document_scope():\n    pass\n",
    )
    .unwrap();
    let documents = Generator::new()
        .generate(
            CodeSource::Directory {
                path: directory.path().to_path_buf(),
            },
            GenerateOptions {
                semantic_analysis: false,
                source_scope: SourceScope::DocumentsOnly,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(documents.human_markdown.contains("Operational guidance."));
    assert!(documents
        .knowledge_base_markdown
        .contains("source_scope: documents_only"));
    assert!(documents.knowledge_base_markdown.contains("guide.md"));
    assert!(!documents
        .knowledge_base_markdown
        .contains("hidden_from_document_scope"));
}

struct MockLlm;

impl LlmClient for MockLlm {
    async fn describe(
        &self,
        _model: &str,
        _base_url: &url::Url,
        _entity: &Entity,
    ) -> Result<SemanticResponse, String> {
        Ok(SemanticResponse {
            purpose: "Parses source input.".into(),
            responsibilities: vec!["parse".into()],
            concepts: vec!["code analysis".into()],
            relationships: vec![],
        })
    }
}

#[tokio::test]
async fn semantic_concepts_are_marked_as_llm_graph_facts() {
    let source = CodeSource::Snippet {
        name: "semantic-example".into(),
        language: Some("rust".into()),
        code: "struct Parser;".into(),
    };
    let documents = Generator::with_llm(MockLlm)
        .generate(
            source,
            GenerateOptions {
                semantic_analysis: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(documents.human_markdown.contains("Parses source input."));
    assert!(documents.knowledge_base_markdown.contains("codeanalysis"));
    assert!(documents
        .knowledge_base_markdown
        .contains("PART_OF_SUBSYSTEM"));
    assert!(documents
        .knowledge_base_markdown
        .contains("layer: complete"));
    assert!(documents
        .knowledge_base_markdown
        .contains("coverage: all_discovered_entities"));
}

struct PrimitiveConceptLlm;

impl LlmClient for PrimitiveConceptLlm {
    async fn describe(
        &self,
        _model: &str,
        _base_url: &url::Url,
        _entity: &Entity,
    ) -> Result<SemanticResponse, String> {
        Ok(SemanticResponse {
            purpose: "Models a domain conflict.".into(),
            responsibilities: vec![],
            concepts: vec![
                "str".into(),
                "Optional".into(),
                "conflict-resolution".into(),
            ],
            relationships: vec![],
        })
    }
}

#[tokio::test]
async fn complete_semantic_output_filters_primitives_and_keeps_all_symbols() {
    let source = CodeSource::Snippet {
        name: "complete".into(),
        language: Some("rust".into()),
        code: "struct First; struct Second;".into(),
    };
    let documents = Generator::with_llm(PrimitiveConceptLlm)
        .generate(
            source,
            GenerateOptions {
                semantic_analysis: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(documents.knowledge_base_markdown.contains("First"));
    assert!(documents.knowledge_base_markdown.contains("Second"));
    assert!(documents
        .knowledge_base_markdown
        .contains("concept:complete.domain.conflictresolution"));
    assert!(!documents
        .knowledge_base_markdown
        .contains("concept:complete.domain.str"));
    assert!(!documents
        .knowledge_base_markdown
        .contains("concept:complete.domain.optional"));
}
