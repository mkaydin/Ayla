use crate::model::{GenerateOptions, GeneratedDocuments, KnowledgePackage};
use std::fmt::Write;

pub(crate) fn render(package: KnowledgePackage, options: &GenerateOptions) -> GeneratedDocuments {
    let project_name = options
        .output_name
        .clone()
        .unwrap_or_else(|| package.project_name.clone());
    let human_markdown = human(&project_name, &package);
    let knowledge_base_markdown = kb(&project_name, &package, options);
    GeneratedDocuments {
        project_name,
        repository_uid: package.repository_uid,
        entities: package.entities,
        relations: package.relations,
        human_markdown,
        knowledge_base_markdown,
        diagnostics: package.diagnostics,
    }
}

fn human(name: &str, package: &KnowledgePackage) -> String {
    let mut text = format!("# {name}\n\n## Overview\n\n");
    let entity_count = package
        .entities
        .iter()
        .filter(|item| {
            item.label != "Repository" && item.label != "File" && item.label != "Dependency"
        })
        .count();
    writeln!(text, "This documentation was generated from deterministic repository discovery, documentation/manifest extraction, and optional source syntax analysis. It identifies {entity_count} project entities.").unwrap();
    if let Some(description) = package
        .entities
        .iter()
        .find(|entity| entity.label == "Repository")
        .and_then(|entity| entity.description.as_ref())
    {
        writeln!(text, "\n{description}").unwrap();
    }
    let mut documentation: Vec<_> = package
        .entities
        .iter()
        .filter(|entity| entity.label == "Documentation")
        .collect();
    if !documentation.is_empty() {
        documentation.sort_by_key(|entity| human_document_priority(entity));
        text.push_str("\n## Key Project Documents\n\n");
        for document in documentation.iter().copied().filter(|entity| {
            entity.origin == "README"
                || entity
                    .properties
                    .get("semanticInterpreted")
                    .and_then(|value| value.as_bool())
                    == Some(true)
        }) {
            writeln!(text, "### {}\n", document.name).unwrap();
            writeln!(
                text,
                "{}\n",
                document
                    .description
                    .as_deref()
                    .unwrap_or("No document summary was available.")
            )
            .unwrap();
            if let Some(location) = &document.source_location {
                writeln!(text, "Source: `{location}`\n").unwrap();
            }
        }
        text.push_str("## Documentation Inventory\n\n");
        for document in documentation {
            let summary = document
                .description
                .as_deref()
                .map(|value| truncate_text(value, 180))
                .unwrap_or_else(|| "No summary available.".into());
            writeln!(text, "- `{}` — {}", document.name, summary).unwrap();
        }
    }
    text.push_str("\n## Architecture\n\n");
    let mut components: Vec<_> = package
        .entities
        .iter()
        .filter(|item| matches!(item.label.as_str(), "Module" | "Struct" | "Trait" | "Class"))
        .filter(|item| !is_test_entity(item))
        .collect();
    components.sort_by_key(|item| importance_key(item));
    if components.is_empty() {
        text.push_str("No top-level architectural components were discovered.\n");
    } else {
        for item in components.into_iter().take(48) {
            writeln!(
                text,
                "- **{}** (`{}`){}{}",
                item.name,
                item.label,
                item.description
                    .as_ref()
                    .map(|v| format!(": {v}"))
                    .unwrap_or_default(),
                item.source_location
                    .as_ref()
                    .map(|v| format!(" — `{v}`"))
                    .unwrap_or_default()
            )
            .unwrap();
        }
    }
    text.push_str("\n## Important Functions and Methods\n\n");
    let mut important: Vec<_> = package
        .entities
        .iter()
        .filter(|item| matches!(item.label.as_str(), "Function" | "Method"))
        .filter(|item| !is_test_entity(item))
        .collect();
    important.sort_by_key(|item| importance_key(item));
    for item in important.into_iter().take(80) {
        writeln!(text, "### {}\n", item.name).unwrap();
        writeln!(
            text,
            "{}\n",
            item.description
                .as_deref()
                .unwrap_or("Purpose was not semantically analyzed.")
        )
        .unwrap();
        if let Some(location) = &item.source_location {
            writeln!(text, "Source: `{location}`\n").unwrap();
        }
    }
    text.push_str("## Declared Dependencies\n\n");
    let dependencies: Vec<_> = package
        .entities
        .iter()
        .filter(|item| matches!(item.label.as_str(), "Dependency" | "DependencyPackage"))
        .collect();
    if dependencies.is_empty() {
        text.push_str("No dependency declarations were discovered.\n");
    } else {
        for item in dependencies.into_iter().take(200) {
            let ecosystem = item.properties.get("ecosystem").and_then(|v| v.as_str());
            writeln!(
                text,
                "- `{}`{}",
                item.name,
                ecosystem.map(|v| format!(" ({v})")).unwrap_or_default()
            )
            .unwrap();
        }
    }
    let licenses: Vec<_> = package
        .entities
        .iter()
        .filter(|item| item.label == "License")
        .collect();
    if !licenses.is_empty() {
        text.push_str("\n## License\n\n");
        for item in licenses {
            writeln!(
                text,
                "- **{}** — `{}`",
                item.name,
                item.source_location.as_deref().unwrap_or("unknown source")
            )
            .unwrap();
        }
    }
    let configs: Vec<_> = package
        .entities
        .iter()
        .filter(|item| item.label == "ConfigFile")
        .collect();
    if !configs.is_empty() {
        text.push_str("\n## Configuration Surface\n\n");
        for item in configs {
            let keys = item
                .properties
                .get("configKeys")
                .and_then(|v| v.as_array())
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            writeln!(text, "- `{}`: {}", item.name, keys).unwrap();
        }
    }
    if !package.diagnostics.is_empty() {
        text.push_str("\n## Analysis Notes\n\n");
        for diagnostic in &package.diagnostics {
            writeln!(text, "- {:?}: {}", diagnostic.level, diagnostic.message).unwrap();
        }
    }
    text
}

fn is_test_entity(entity: &crate::Entity) -> bool {
    entity.source_location.as_deref().is_some_and(|location| {
        location.contains("/tests/") || location.starts_with("tests/") || location.contains("test_")
    })
}

fn importance_key(entity: &crate::Entity) -> (u8, String) {
    let location = entity.source_location.as_deref().unwrap_or("");
    let rank = if location.contains("backend/main.py") || entity.name == "main" {
        0
    } else if ["/api/", "/agents/", "/rag/", "/db/", "/ingestion/"]
        .iter()
        .any(|part| location.contains(part))
    {
        1
    } else if location.contains("/scripts/") || location.contains("start.py") {
        3
    } else {
        2
    };
    (rank, location.to_owned())
}

fn human_document_priority(entity: &crate::Entity) -> (u8, String) {
    let interpreted = entity
        .properties
        .get("semanticInterpreted")
        .and_then(|value| value.as_bool())
        == Some(true);
    let rank = if entity.origin == "README" && !entity.name.contains('/') {
        0
    } else if interpreted {
        1
    } else if entity.origin == "README" {
        2
    } else {
        3
    };
    (rank, entity.name.clone())
}

fn truncate_text(value: &str, max_characters: usize) -> String {
    let mut text = value.chars().take(max_characters).collect::<String>();
    if value.chars().count() > max_characters {
        text.push('…');
    }
    text
}

fn kb(name: &str, package: &KnowledgePackage, options: &GenerateOptions) -> String {
    let semantic_candidates = package
        .entities
        .iter()
        .filter(|entity| is_code_entity(entity))
        .count();
    let semantic_complete = package
        .entities
        .iter()
        .filter(|entity| is_code_entity(entity) && entity.description.is_some())
        .count();
    let documentation_count = package
        .entities
        .iter()
        .filter(|entity| entity.label == "Documentation")
        .count();
    let documentation_semantic_complete = package
        .entities
        .iter()
        .filter(|entity| {
            entity.label == "Documentation"
                && entity
                    .properties
                    .get("semanticInterpreted")
                    .and_then(|value| value.as_bool())
                    == Some(true)
        })
        .count();
    let source_scope = match options.source_scope {
        crate::SourceScope::All => "all",
        crate::SourceScope::DocumentsOnly => "documents_only",
    };
    let source_type = if options.source_scope == crate::SourceScope::DocumentsOnly {
        "repository_documentation"
    } else {
        "source_code"
    };
    let semantic_target = if options.semantic_analysis {
        options
            .semantic_symbol_limit
            .unwrap_or(semantic_candidates)
            .min(semantic_candidates)
    } else {
        0
    };
    let documentation_semantic_target = if options.semantic_analysis {
        options
            .semantic_document_limit
            .unwrap_or(documentation_count)
            .min(documentation_count)
    } else {
        0
    };
    let semantic_scope = if !options.semantic_analysis {
        "disabled"
    } else if options.semantic_symbol_limit.is_none() {
        "all"
    } else {
        "prioritized"
    };
    let mut text = format!("# Knowledge Base Schema & Content: {name}\n\n---\nschema_version: 2\nsource_type: {}\nsource_scope: {}\nlayer: complete\ncoverage: all_discovered_entities\nrepository_uid: {}\nsemantic_model: {}\nsemantic_enabled: {}\nsemantic_scope: {}\ncode_semantic_eligible: {}\ncode_semantic_target: {}\ncode_semantic_coverage: \"{}/{}\"\ndocumentation_count: {}\ndocumentation_semantic_target: {}\ndocumentation_semantic_coverage: \"{}/{}\"\nentity_count: {}\nrelation_count: {}\n---\n\n", source_type, source_scope, package.repository_uid, options.model, options.semantic_analysis, semantic_scope, semantic_candidates, semantic_target, semantic_complete.min(semantic_target), semantic_target, documentation_count, documentation_semantic_target, documentation_semantic_complete.min(documentation_semantic_target), documentation_semantic_target, package.entities.len(), package.relations.len());
    text.push_str(concat!(
        "## 1. Graph Ontology Definition\n\n",
        "### Node Labels & Properties\n\n",
        "| Node Label | Description | Required Properties |\n",
        "| :--- | :--- | :--- |\n",
        "| `Repository` | The analyzed source collection | `uid`, `name` |\n",
        "| `File`, `InfrastructureFile`, `Documentation`, `Manifest`, `ConfigFile`, `License` | Source, documentation, manifest, configuration, infrastructure, or license files | `uid`, `name`, `sourceLocation` |\n",
        "| `Module`, `Struct`, `Trait`, `Enum`, `Class`, `Function`, `Method` | Parsed code entities | `uid`, `name`, `sourceLocation`, `origin`, `confidence`, `isTest`, `semanticStatus` |\n",
        "| `Dependency`, `DependencyPackage` | Imported or declared dependency identities | `uid`, `name`, `ecosystem` |\n",
        "| `ContainerService`, `ContainerImage` | Container infrastructure | `uid`, `name`, `origin` |\n",
        "| `ConfigKey` | Declared configuration key; secret values are never stored | `uid`, `name` |\n",
        "| `Technique`, `Tactic`, `Tool` | Canonical external technique/tactic identity or documented tool | `uid`, `name`, `authority` |\n",
        "| `SystemOverview` | Concise architecture anchor | `uid`, `description` |\n",
        "| `Concept` | Namespaced semantic concept | `uid`, `name`, `origin`, `confidence` |\n\n",
        "### Edge Types (Relations)\n\n",
        "| Edge Type | Source → Target |\n",
        "| :--- | :--- |\n",
        "| `CONTAINS` | Repository → discovered file-backed node |\n",
        "| `DEFINES` | File or symbol → symbol |\n",
        "| `IMPORTS` | File or symbol → Dependency |\n",
        "| `PART_OF_SUBSYSTEM` | Code entity → Concept |\n",
        "| `USES_CONTAINER` | ContainerService → ContainerImage |\n",
        "| `DOCUMENTED_BY` | Repository → Documentation |\n",
        "| `DECLARES_DEPENDENCY` | Manifest → DependencyPackage |\n",
        "| `MENTIONS_DEPENDENCY` | Documentation → DependencyPackage |\n",
        "| `DECLARES_CONFIG_KEY` | ConfigFile → ConfigKey |\n",
        "| `USES_CONFIG_KEY` | ContainerService → ConfigKey |\n",
        "| `MENTIONS`, `ADDRESSES_TECHNIQUE` | Documentation → Technique |\n",
        "| `PART_OF_TACTIC` | Technique → Tactic |\n",
        "| `USES_TECHNIQUE` | Tool → Technique |\n",
        "| `RELATED_TO`, `PART_OF`, `ENABLES`, `USES`, `IMPLEMENTS`, `MITIGATES`, `SUPPORTS` | Concept → Concept |\n",
        "| `LICENSED_UNDER` | Repository → License |\n",
        "| `HAS_OVERVIEW` | Repository → SystemOverview |\n\n",
        "## 2. Core Knowledge Nodes\n\n"
    ));
    let included: Vec<_> = package.entities.iter().collect();
    let included_ids: std::collections::BTreeSet<_> =
        included.iter().map(|entity| entity.uid.as_str()).collect();
    let mut labels: Vec<_> = included
        .iter()
        .map(|entity| entity.label.as_str())
        .collect();
    labels.sort();
    labels.dedup();
    for label in labels {
        writeln!(text, "### {label} (`{label}`)\n").unwrap();
        for entity in included
            .iter()
            .copied()
            .filter(|entity| entity.label == label)
        {
            writeln!(
                text,
                "- **{}:** uid: `{}`. {}origin: {}. confidence: {:.2}. isTest: {}. semanticStatus: {}.{}{}{}",
                entity.name,
                entity.uid,
                entity
                    .description
                    .as_ref()
                    .map(|value| format!("{value}. "))
                    .unwrap_or_default(),
                entity.origin,
                entity.confidence,
                is_test_entity(entity),
                semantic_status(entity),
                entity
                    .source_location
                    .as_ref()
                    .map(|v| format!(" sourceLocation: `{v}`."))
                    .unwrap_or_default(),
                entity
                    .embedding_text
                    .as_ref()
                    .map(|v| format!(" embeddingText: {v}"))
                    .unwrap_or_default(),
                if entity.properties.is_empty() {
                    String::new()
                } else {
                    format!(" properties: {}", json_properties(&entity.properties))
                }
            )
            .unwrap();
        }
        text.push('\n');
    }
    text.push_str("## 3. Canonical Relations\n\n| Source.uid | Relation | Target.uid | Origin | Confidence | Source Location | Properties |\n| :--- | :--- | :--- | :--- | :--- | :--- | :--- |\n");
    for relation in package.relations.iter().filter(|relation| {
        included_ids.contains(relation.source.as_str())
            && included_ids.contains(relation.target.as_str())
    }) {
        writeln!(
            text,
            "| `{}` | `{}` | `{}` | {} | {:.2} | {} | {} |",
            relation.source,
            relation.relation,
            relation.target,
            relation.origin,
            relation.confidence,
            relation.source_location.as_deref().unwrap_or(""),
            json_properties(&relation.properties).replace('|', "\\|")
        )
        .unwrap();
    }
    text.push_str("\n## 4. Maintenance Guidelines\n\n1. Treat `uid` as the stable merge key within this repository fingerprint.\n2. Preserve `origin` and `confidence`; AST facts must not be overwritten by LLM inference.\n3. Use `embeddingText` for vector retrieval and retain source locations for auditability.\n");
    text
}

fn json_properties(properties: &std::collections::BTreeMap<String, serde_json::Value>) -> String {
    serde_json::to_string(properties).unwrap_or_else(|_| "{}".into())
}

fn is_code_entity(entity: &crate::Entity) -> bool {
    matches!(
        entity.label.as_str(),
        "Class" | "Struct" | "Trait" | "Enum" | "Module" | "Function" | "Method"
    )
}

fn semantic_status(entity: &crate::Entity) -> &'static str {
    if entity
        .properties
        .get("semanticInterpreted")
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        "interpreted"
    } else if is_code_entity(entity) {
        "structural-fallback"
    } else if entity.label == "Documentation" {
        "deterministic-summary"
    } else {
        "not-applicable"
    }
}
