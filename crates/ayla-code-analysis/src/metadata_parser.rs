use crate::model::{Entity, KnowledgePackage, Relation};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) fn is_structured_file(path: &str) -> bool {
    let name = file_name(path).to_ascii_lowercase();
    name == "package.json"
        || name == "pyproject.toml"
        || name == "cargo.toml"
        || name == "go.mod"
        || name.starts_with("requirements") && name.ends_with(".txt")
        || matches!(
            name.as_str(),
            ".env.example" | ".env.sample" | ".env.template"
        )
        || is_license(path)
}

pub(crate) fn analyze_structured_file(p: &mut KnowledgePackage, file: &str, source: &str) {
    let lower = file_name(file).to_ascii_lowercase();
    if lower.starts_with("requirements") && lower.ends_with(".txt") {
        analyze_requirements(p, file, source);
    } else if lower == "package.json" {
        analyze_package_json(p, file, source);
    } else if lower == "pyproject.toml" {
        analyze_pyproject(p, file, source);
    } else if lower == "cargo.toml" {
        analyze_cargo(p, file, source);
    } else if lower == "go.mod" {
        analyze_go_mod(p, file, source);
    } else if lower.starts_with(".env.") {
        analyze_env_template(p, file, source);
    } else if is_license(file) {
        analyze_license(p, file, source);
    }
}

pub(crate) fn enrich_document(p: &mut KnowledgePackage, file: &str, source: &str) {
    let document_uid = format!("{}:documentation:{file}", p.repository_uid);
    let evidence = format!("{file}\n{source}");
    let ids = mitre_ids(&evidence);
    let tactics = explicit_metadata_values(source, "tactic");
    let tools = explicit_metadata_values(source, "tool");
    for id in ids {
        let uid = format!("external:mitre-attack:{id}");
        ensure_entity(
            p,
            Entity {
                uid: uid.clone(),
                label: "Technique".into(),
                name: id.clone(),
                description: Some(format!("MITRE ATT&CK technique or sub-technique {id}.")),
                source_location: None,
                origin: "DOCUMENT".into(),
                confidence: 1.0,
                embedding_text: Some(format!("MITRE ATT&CK technique {id}.")),
                properties: properties([
                    ("externalId", json!(id)),
                    ("authority", json!("MITRE ATT&CK")),
                ]),
                source_excerpt: None,
            },
        );
        add_relation(
            p,
            &document_uid,
            "MENTIONS",
            &uid,
            "DOCUMENT",
            file,
            BTreeMap::new(),
        );
        if is_playbook(file, source) {
            add_relation(
                p,
                &document_uid,
                "ADDRESSES_TECHNIQUE",
                &uid,
                "DOCUMENT",
                file,
                BTreeMap::new(),
            );
        }
        for tactic in &tactics {
            let tactic_uid = format!(
                "external:mitre-attack:tactic:{}",
                crate::model::safe_file_stem(tactic)
            );
            ensure_entity(
                p,
                Entity {
                    uid: tactic_uid.clone(),
                    label: "Tactic".into(),
                    name: tactic.clone(),
                    description: Some(format!("MITRE ATT&CK tactic {tactic}.")),
                    source_location: None,
                    origin: "DOCUMENT".into(),
                    confidence: 1.0,
                    embedding_text: Some(format!("MITRE ATT&CK tactic {tactic}.")),
                    properties: properties([("authority", json!("MITRE ATT&CK"))]),
                    source_excerpt: None,
                },
            );
            add_relation(
                p,
                &uid,
                "PART_OF_TACTIC",
                &tactic_uid,
                "DOCUMENT",
                file,
                BTreeMap::new(),
            );
        }
        for tool in &tools {
            let tool_uid = format!(
                "tool:{}.{}",
                crate::model::safe_file_stem(&p.project_name),
                crate::model::safe_file_stem(tool)
            );
            ensure_entity(
                p,
                Entity {
                    uid: tool_uid.clone(),
                    label: "Tool".into(),
                    name: tool.clone(),
                    description: Some(format!("Tool referenced by project documentation: {tool}.")),
                    source_location: None,
                    origin: "DOCUMENT".into(),
                    confidence: 1.0,
                    embedding_text: Some(format!("Tool {tool}.")),
                    properties: properties([("sourceLocations", json!([file]))]),
                    source_excerpt: None,
                },
            );
            add_relation(
                p,
                &tool_uid,
                "USES_TECHNIQUE",
                &uid,
                "DOCUMENT",
                file,
                BTreeMap::new(),
            );
        }
    }
}

pub(crate) fn add_system_overview(p: &mut KnowledgePackage) {
    let documents = p
        .entities
        .iter()
        .filter(|e| e.label == "Documentation")
        .count();
    let dependencies = p
        .entities
        .iter()
        .filter(|e| e.label == "DependencyPackage")
        .count();
    let services: Vec<_> = p
        .entities
        .iter()
        .filter(|e| e.label == "ContainerService")
        .map(|e| e.name.clone())
        .collect();
    let repository_description = p
        .entities
        .iter()
        .find(|e| e.uid == p.repository_uid)
        .and_then(|e| e.description.clone());
    let mut summary = repository_description.unwrap_or_else(|| {
        format!(
            "{} is a repository represented by its project documentation and metadata.",
            p.project_name
        )
    });
    summary.push_str(&format!(
        " The documentation graph contains {documents} documents and {dependencies} declared dependency packages."
    ));
    if !services.is_empty() {
        summary.push_str(&format!(" Container services: {}.", services.join(", ")));
    }
    let uid = format!("{}:system-overview", p.repository_uid);
    p.add_entity(Entity {
        uid: uid.clone(),
        label: "SystemOverview".into(),
        name: format!("{} system overview", p.project_name),
        description: Some(summary.clone()),
        source_location: None,
        origin: "SYNTHESIS".into(),
        confidence: 0.9,
        embedding_text: Some(summary),
        properties: properties([
            ("documentationCount", json!(documents)),
            ("dependencyCount", json!(dependencies)),
            ("containerServices", json!(services)),
        ]),
        source_excerpt: None,
    });
    add_relation(
        p,
        &p.repository_uid.clone(),
        "HAS_OVERVIEW",
        &uid,
        "SYNTHESIS",
        "",
        BTreeMap::new(),
    );
}

fn analyze_requirements(p: &mut KnowledgePackage, file: &str, source: &str) {
    let manifest = add_manifest(p, file, "pip-requirements", "Python dependency manifest");
    for (line_index, raw) in source.lines().enumerate() {
        let declaration = raw.split('#').next().unwrap_or("").trim();
        if declaration.is_empty() || declaration.starts_with('-') {
            continue;
        }
        if let Some((name, constraint)) = parse_python_requirement(declaration) {
            add_dependency(
                p,
                &manifest,
                file,
                line_index + 1,
                "pypi",
                &name,
                constraint.as_deref(),
                "runtime",
                declaration,
            );
        }
    }
}

fn analyze_package_json(p: &mut KnowledgePackage, file: &str, source: &str) {
    let manifest = add_manifest(p, file, "npm-package", "Node.js package manifest");
    let Ok(value) = serde_json::from_str::<Value>(source) else {
        return;
    };
    for (field, scope) in [
        ("dependencies", "runtime"),
        ("devDependencies", "development"),
        ("peerDependencies", "peer"),
    ] {
        if let Some(entries) = value.get(field).and_then(Value::as_object) {
            for (name, constraint) in entries {
                add_dependency(
                    p,
                    &manifest,
                    file,
                    0,
                    "npm",
                    name,
                    constraint.as_str(),
                    scope,
                    &format!("{name}: {constraint}"),
                );
            }
        }
    }
}

fn analyze_pyproject(p: &mut KnowledgePackage, file: &str, source: &str) {
    let manifest = add_manifest(p, file, "pyproject", "Python project manifest");
    let mut section = String::new();
    let mut in_project_dependencies = false;
    for (line_index, raw) in source.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']']).to_ascii_lowercase();
            in_project_dependencies = false;
            continue;
        }
        if section == "project" && line.starts_with("dependencies") && line.contains('[') {
            in_project_dependencies = true;
        }
        if in_project_dependencies {
            for quoted in quoted_values(line) {
                if let Some((name, constraint)) = parse_python_requirement(&quoted) {
                    add_dependency(
                        p,
                        &manifest,
                        file,
                        line_index + 1,
                        "pypi",
                        &name,
                        constraint.as_deref(),
                        "runtime",
                        &quoted,
                    );
                }
            }
            if line.contains(']') {
                in_project_dependencies = false;
            }
        } else if matches!(
            section.as_str(),
            "tool.poetry.dependencies" | "tool.poetry.group.dev.dependencies"
        ) {
            if let Some((name, constraint)) = line.split_once('=') {
                let name = name.trim();
                if name != "python" && !name.is_empty() {
                    let constraint = constraint.trim().trim_matches(['\'', '"']);
                    let scope = if section.contains(".dev.") {
                        "development"
                    } else {
                        "runtime"
                    };
                    add_dependency(
                        p,
                        &manifest,
                        file,
                        line_index + 1,
                        "pypi",
                        name,
                        Some(constraint),
                        scope,
                        line,
                    );
                }
            }
        }
    }
}

fn analyze_cargo(p: &mut KnowledgePackage, file: &str, source: &str) {
    let manifest = add_manifest(p, file, "cargo", "Rust package manifest");
    let mut section = "";
    for (line_index, raw) in source.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']']);
            continue;
        }
        if matches!(
            section,
            "dependencies" | "dev-dependencies" | "build-dependencies"
        ) {
            if let Some((name, value)) = line.split_once('=') {
                let name = name.trim();
                if !name.is_empty() {
                    let value = value.trim();
                    let constraint = if value.starts_with('{') {
                        extract_inline_field(value, "version")
                    } else {
                        Some(value.trim_matches(['\'', '"']).to_owned())
                    };
                    add_dependency(
                        p,
                        &manifest,
                        file,
                        line_index + 1,
                        "crates.io",
                        name,
                        constraint.as_deref(),
                        if section == "dependencies" {
                            "runtime"
                        } else {
                            section
                        },
                        line,
                    );
                }
            }
        }
    }
}

fn analyze_go_mod(p: &mut KnowledgePackage, file: &str, source: &str) {
    let manifest = add_manifest(p, file, "go-module", "Go module dependency manifest");
    let mut require_block = false;
    for (line_index, raw) in source.lines().enumerate() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line == "require (" {
            require_block = true;
            continue;
        }
        if require_block && line == ")" {
            require_block = false;
            continue;
        }
        let candidate = if require_block {
            line
        } else {
            line.strip_prefix("require ").unwrap_or("")
        };
        let parts: Vec<_> = candidate.split_whitespace().collect();
        if parts.len() >= 2 {
            add_dependency(
                p,
                &manifest,
                file,
                line_index + 1,
                "go",
                parts[0],
                Some(parts[1]),
                "runtime",
                candidate,
            );
        }
    }
}

fn analyze_env_template(p: &mut KnowledgePackage, file: &str, source: &str) {
    let file_uid = format!("{}:config:{file}", p.repository_uid);
    let keys: Vec<_> = source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let key = line
                .strip_prefix("export ")
                .unwrap_or(line)
                .split_once('=')?
                .0
                .trim();
            if key
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                Some(key.to_owned())
            } else {
                None
            }
        })
        .collect();
    p.add_entity(Entity {
        uid: file_uid.clone(),
        label: "ConfigFile".into(),
        name: file.into(),
        description: Some(
            "Environment variable template; values are intentionally not ingested.".into(),
        ),
        source_location: Some(file.into()),
        origin: "CONFIG".into(),
        confidence: 1.0,
        embedding_text: Some(format!(
            "Environment configuration template defining keys: {}.",
            keys.join(", ")
        )),
        properties: properties([
            ("configKeys", json!(keys)),
            ("valuesIngested", json!(false)),
        ]),
        source_excerpt: None,
    });
    add_relation(
        p,
        &p.repository_uid.clone(),
        "CONTAINS",
        &file_uid,
        "CONFIG",
        file,
        BTreeMap::new(),
    );
    for key in keys {
        let uid = format!(
            "{}:config-key:{}",
            p.repository_uid,
            key.to_ascii_lowercase()
        );
        ensure_entity(
            p,
            Entity {
                uid: uid.clone(),
                label: "ConfigKey".into(),
                name: key.clone(),
                description: Some(format!("Configuration key {key}.")),
                source_location: None,
                origin: "CONFIG".into(),
                confidence: 1.0,
                embedding_text: Some(format!("Configuration key {key}.")),
                properties: properties([("secretValueStored", json!(false))]),
                source_excerpt: None,
            },
        );
        add_relation(
            p,
            &file_uid,
            "DECLARES_CONFIG_KEY",
            &uid,
            "CONFIG",
            file,
            BTreeMap::new(),
        );
    }
}

fn analyze_license(p: &mut KnowledgePackage, file: &str, source: &str) {
    let kind = detect_license(source);
    let copyright = source
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("copyright"))
        .map(str::trim)
        .unwrap_or("");
    let years: Vec<_> = copyright
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| s.len() == 4 && matches!(s.parse::<u16>(), Ok(1900..=2100)))
        .collect();
    let uid = format!("{}:license:{file}", p.repository_uid);
    p.add_entity(Entity {
        uid: uid.clone(),
        label: "License".into(),
        name: kind.clone(),
        description: Some(format!("Repository license declared by {file}.")),
        source_location: Some(file.into()),
        origin: "LICENSE".into(),
        confidence: if kind == "Unknown" { 0.5 } else { 1.0 },
        embedding_text: Some(format!("Project license: {kind}. {copyright}")),
        properties: properties([
            ("licenseType", json!(kind)),
            ("copyrightNotice", json!(copyright)),
            ("years", json!(years)),
        ]),
        source_excerpt: None,
    });
    add_relation(
        p,
        &p.repository_uid.clone(),
        "CONTAINS",
        &uid,
        "LICENSE",
        file,
        BTreeMap::new(),
    );
    add_relation(
        p,
        &p.repository_uid.clone(),
        "LICENSED_UNDER",
        &uid,
        "LICENSE",
        file,
        BTreeMap::new(),
    );
}

fn add_manifest(p: &mut KnowledgePackage, file: &str, kind: &str, description: &str) -> String {
    let uid = format!("{}:manifest:{file}", p.repository_uid);
    p.add_entity(Entity {
        uid: uid.clone(),
        label: "Manifest".into(),
        name: file.into(),
        description: Some(description.into()),
        source_location: Some(file.into()),
        origin: "MANIFEST".into(),
        confidence: 1.0,
        embedding_text: Some(format!("{description}: {file}.")),
        properties: properties([("manifestType", json!(kind))]),
        source_excerpt: None,
    });
    add_relation(
        p,
        &p.repository_uid.clone(),
        "CONTAINS",
        &uid,
        "MANIFEST",
        file,
        BTreeMap::new(),
    );
    uid
}

#[allow(clippy::too_many_arguments)]
fn add_dependency(
    p: &mut KnowledgePackage,
    manifest: &str,
    file: &str,
    line: usize,
    ecosystem: &str,
    name: &str,
    constraint: Option<&str>,
    scope: &str,
    raw: &str,
) {
    let canonical = canonical_package_name(ecosystem, name);
    if canonical.is_empty() {
        return;
    }
    let uid = format!("package:{ecosystem}:{canonical}");
    ensure_entity(
        p,
        Entity {
            uid: uid.clone(),
            label: "DependencyPackage".into(),
            name: name.into(),
            description: Some(format!("{ecosystem} dependency package {name}.")),
            source_location: None,
            origin: "MANIFEST".into(),
            confidence: 1.0,
            embedding_text: Some(format!("{name} package in the {ecosystem} ecosystem.")),
            properties: properties([
                ("ecosystem", json!(ecosystem)),
                ("canonicalName", json!(canonical)),
            ]),
            source_excerpt: None,
        },
    );
    let mut edge = properties([("scope", json!(scope)), ("rawDeclaration", json!(raw))]);
    if let Some(value) = constraint.filter(|v| !v.is_empty()) {
        edge.insert("versionConstraint".into(), json!(value));
    }
    let location = if line == 0 {
        file.to_owned()
    } else {
        format!("{file}:{line}")
    };
    add_relation(
        p,
        manifest,
        "DECLARES_DEPENDENCY",
        &uid,
        "MANIFEST",
        &location,
        edge,
    );
}

fn add_relation(
    p: &mut KnowledgePackage,
    source: &str,
    relation: &str,
    target: &str,
    origin: &str,
    location: &str,
    properties: BTreeMap<String, Value>,
) {
    p.add_relation(Relation {
        source: source.into(),
        relation: relation.into(),
        target: target.into(),
        origin: origin.into(),
        confidence: 1.0,
        source_location: (!location.is_empty()).then(|| location.into()),
        properties,
    });
}

fn ensure_entity(p: &mut KnowledgePackage, entity: Entity) {
    if !p
        .entities
        .iter()
        .any(|candidate| candidate.uid == entity.uid)
    {
        p.add_entity(entity);
    }
}

fn properties<const N: usize>(entries: [(&str, Value); N]) -> BTreeMap<String, Value> {
    entries
        .into_iter()
        .map(|(key, value)| (key.into(), value))
        .collect()
}

fn parse_python_requirement(value: &str) -> Option<(String, Option<String>)> {
    let declaration = value.split(';').next()?.trim();
    let split = declaration
        .find(|c: char| matches!(c, '<' | '>' | '=' | '!' | '~' | '@'))
        .unwrap_or(declaration.len());
    let raw_name = declaration[..split].trim();
    let name = raw_name.split('[').next()?.trim();
    if name.is_empty() {
        return None;
    }
    let constraint = declaration[split..].trim();
    Some((
        name.into(),
        (!constraint.is_empty()).then(|| constraint.into()),
    ))
}

fn quoted_values(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut quote = None;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        if matches!(character, '\'' | '"') {
            if quote == Some(character) {
                result.push(value[start..index].to_owned());
                quote = None;
            } else if quote.is_none() {
                quote = Some(character);
                start = index + character.len_utf8();
            }
        }
    }
    result
}

fn extract_inline_field(value: &str, field: &str) -> Option<String> {
    let tail = value
        .split(',')
        .find(|part| part.trim_start().starts_with(field))?;
    Some(
        tail.split_once('=')?
            .1
            .trim()
            .trim_matches(['\'', '"', ' ', '}'])
            .to_owned(),
    )
}

fn canonical_package_name(ecosystem: &str, name: &str) -> String {
    let lowered = name.trim().to_ascii_lowercase();
    if ecosystem == "pypi" {
        lowered.replace(['_', '.'], "-")
    } else {
        lowered
    }
}

fn mitre_ids(source: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for token in source.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.')) {
        let valid = token.starts_with('T')
            && token
                .get(1..5)
                .is_some_and(|v| v.len() == 4 && v.chars().all(|c| c.is_ascii_digit()))
            && (token.len() == 5
                || token.get(5..).is_some_and(|v| {
                    v.len() == 4 && v.starts_with('.') && v[1..].chars().all(|c| c.is_ascii_digit())
                }));
        if valid && !ids.iter().any(|id| id == token) {
            ids.push(token.to_owned());
        }
    }
    ids
}

fn explicit_metadata_values(source: &str, field: &str) -> Vec<String> {
    let mut values = Vec::new();
    for raw in source.lines() {
        let line = raw
            .trim()
            .trim_start_matches(['#', '-', '*', ' '])
            .trim_end_matches('*')
            .trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if key.trim_matches('*').trim().eq_ignore_ascii_case(field) {
            for value in value.split(',') {
                let value = value.trim().trim_matches(['*', '`', '[', ']']);
                if !value.is_empty() && !values.iter().any(|entry| entry == value) {
                    values.push(value.to_owned());
                }
            }
        }
    }
    values
}

fn is_playbook(file: &str, source: &str) -> bool {
    let text =
        format!("{file} {}", source.chars().take(2_000).collect::<String>()).to_ascii_lowercase();
    [
        "playbook",
        "mitigation",
        "remediation",
        "response procedure",
        "incident response",
    ]
    .iter()
    .any(|word| text.contains(word))
}

fn is_license(path: &str) -> bool {
    file_name(path).to_ascii_lowercase().starts_with("license")
}

fn detect_license(source: &str) -> String {
    let lower = source.to_ascii_lowercase();
    if lower.contains("mit license")
        || lower.contains("permission is hereby granted, free of charge")
    {
        "MIT"
    } else if lower.contains("apache license") && lower.contains("version 2.0") {
        "Apache-2.0"
    } else if lower.contains("gnu general public license") && lower.contains("version 3") {
        "GPL-3.0"
    } else if lower.contains("gnu general public license") {
        "GPL"
    } else if lower.contains("bsd 3-clause") {
        "BSD-3-Clause"
    } else {
        "Unknown"
    }
    .into()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
