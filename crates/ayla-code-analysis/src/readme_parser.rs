use crate::model::{Entity, GenerateError, KnowledgePackage, Relation};

pub(crate) fn is_readme(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .is_some_and(|name| name.to_ascii_lowercase().starts_with("readme"))
}

pub(crate) fn is_document(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    let extension = name.rsplit_once('.').map(|(_, extension)| extension);
    is_readme(path)
        || matches!(
            extension,
            Some("md" | "mdx" | "rst" | "txt" | "adoc" | "asciidoc")
        )
        || [
            "license",
            "changelog",
            "contributing",
            "security",
            "code_of_conduct",
            "authors",
            "notice",
        ]
        .iter()
        .any(|prefix| name == *prefix || name.starts_with(&format!("{prefix}.")))
}

pub(crate) fn analyze_document(
    package: &mut KnowledgePackage,
    file: &str,
    source: &str,
) -> Result<(), GenerateError> {
    let uid = format!("{}:documentation:{file}", package.repository_uid);
    let is_readme = is_readme(file);
    let origin = if is_readme { "README" } else { "DOCUMENT" };
    let overview = extract_overview(source);
    let embedding = normalize_for_embedding(source);
    let excerpt = source.chars().take(3_000).collect::<String>();
    package.add_entity(Entity {
        uid: uid.clone(),
        label: "Documentation".into(),
        name: file.into(),
        description: Some(overview.clone()),
        source_location: Some(file.into()),
        origin: origin.into(),
        confidence: 1.0,
        embedding_text: Some(embedding.clone()),
        properties: Default::default(),
        source_excerpt: Some(excerpt.clone()),
    });
    package.add_relation(Relation {
        source: package.repository_uid.clone(),
        relation: "DOCUMENTED_BY".into(),
        target: uid,
        origin: origin.into(),
        confidence: 1.0,
        source_location: Some(file.into()),
        properties: Default::default(),
    });
    package.add_relation(Relation {
        source: package.repository_uid.clone(),
        relation: "CONTAINS".into(),
        target: format!("{}:documentation:{file}", package.repository_uid),
        origin: origin.into(),
        confidence: 1.0,
        source_location: Some(file.into()),
        properties: Default::default(),
    });

    let is_root = is_readme && !file.contains('/');
    if let Some(repository) = package
        .entities
        .iter_mut()
        .find(|entity| entity.uid == package.repository_uid)
    {
        if is_root || repository.description.is_none() {
            repository.description = Some(overview);
            repository.embedding_text = Some(embedding);
            repository.source_excerpt = Some(excerpt);
        }
    }
    Ok(())
}

fn extract_overview(source: &str) -> String {
    let mut paragraph = Vec::new();
    for line in source.lines() {
        let value = line.trim();
        if value.is_empty() {
            if !paragraph.is_empty() {
                break;
            }
            continue;
        }
        if value.starts_with('#')
            || value.starts_with("![")
            || value.starts_with("[![")
            || value.starts_with("<img")
        {
            continue;
        }
        paragraph.push(value);
        if paragraph.join(" ").chars().count() >= 700 {
            break;
        }
    }
    let value = paragraph.join(" ");
    if value.is_empty() {
        "Repository documentation from README.".into()
    } else {
        value.chars().take(1_000).collect()
    }
}

fn normalize_for_embedding(source: &str) -> String {
    source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(4_000)
        .collect()
}
