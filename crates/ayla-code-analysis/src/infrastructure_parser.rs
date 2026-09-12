use crate::model::{Entity, GenerateError, KnowledgePackage, Relation};
use serde_json::json;

pub(crate) fn is_compose_file(path: &str) -> bool {
    matches!(
        path.rsplit('/').next().unwrap_or(path),
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml"
    )
}

pub(crate) fn analyze_compose(
    p: &mut KnowledgePackage,
    file: &str,
    source: &str,
) -> Result<(), GenerateError> {
    let file_uid = format!("{}:file:{file}", p.repository_uid);
    p.add_entity(Entity {
        uid: file_uid.clone(),
        label: "InfrastructureFile".into(),
        name: file.into(),
        description: Some("Docker Compose configuration.".into()),
        source_location: Some(file.into()),
        origin: "MANIFEST".into(),
        confidence: 1.0,
        embedding_text: None,
        properties: [("format".into(), json!("docker-compose"))]
            .into_iter()
            .collect(),
        source_excerpt: None,
    });
    p.add_relation(rel(&p.repository_uid, "CONTAINS", &file_uid, file));
    let mut services = false;
    let mut current: Option<String> = None;
    let mut environment_indent: Option<usize> = None;
    for (index, line) in source.lines().enumerate() {
        let indent = line.len() - line.trim_start().len();
        let value = line.trim();
        if value == "services:" {
            services = true;
            continue;
        }
        if services && indent == 0 && !value.is_empty() {
            services = false;
            current = None;
            environment_indent = None;
        }
        if services && indent == 2 && value.ends_with(':') {
            let name = value.trim_end_matches(':').to_owned();
            let uid = format!("{}:container-service:{name}", p.repository_uid);
            let loc = format!("{}:{}", file, index + 1);
            p.add_entity(Entity {
                uid: uid.clone(),
                label: "ContainerService".into(),
                name: name.clone(),
                description: Some("Docker Compose service.".into()),
                source_location: Some(loc.clone()),
                origin: "MANIFEST".into(),
                confidence: 1.0,
                embedding_text: Some(format!("Docker Compose service {name}.")),
                properties: [("serviceName".into(), json!(name))].into_iter().collect(),
                source_excerpt: None,
            });
            p.add_relation(rel(&file_uid, "DEFINES", &uid, &loc));
            current = Some(uid);
            environment_indent = None;
        } else if services && indent >= 4 && value.starts_with("image:") {
            if let Some(service) = &current {
                let image = value
                    .trim_start_matches("image:")
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                let uid = format!("{}:container-image:{image}", p.repository_uid);
                let (repository, tag) = split_image_reference(image);
                p.add_entity(Entity {
                    uid: uid.clone(),
                    label: "ContainerImage".into(),
                    name: image.into(),
                    description: Some("Docker image used by a Compose service.".into()),
                    source_location: Some(format!("{}:{}", file, index + 1)),
                    origin: "MANIFEST".into(),
                    confidence: 1.0,
                    embedding_text: None,
                    properties: [
                        ("imageReference".into(), json!(image)),
                        ("repository".into(), json!(repository)),
                        ("tag".into(), json!(tag)),
                    ]
                    .into_iter()
                    .collect(),
                    source_excerpt: None,
                });
                p.add_relation(rel(
                    service,
                    "USES_CONTAINER",
                    &uid,
                    &format!("{}:{}", file, index + 1),
                ));
            }
        } else if services && current.is_some() && value == "environment:" {
            environment_indent = Some(indent);
        } else if let (Some(service), Some(environment_level)) = (&current, environment_indent) {
            if indent <= environment_level && !value.is_empty() {
                environment_indent = None;
            } else if let Some(key) = compose_environment_key(value) {
                let uid = format!(
                    "{}:config-key:{}",
                    p.repository_uid,
                    key.to_ascii_lowercase()
                );
                p.add_entity(Entity {
                    uid: uid.clone(),
                    label: "ConfigKey".into(),
                    name: key.clone(),
                    description: Some(format!("Container configuration key {key}.")),
                    source_location: None,
                    origin: "MANIFEST".into(),
                    confidence: 1.0,
                    embedding_text: Some(format!("Container configuration key {key}.")),
                    properties: [("secretValueStored".into(), json!(false))]
                        .into_iter()
                        .collect(),
                    source_excerpt: None,
                });
                p.add_relation(rel(
                    service,
                    "USES_CONFIG_KEY",
                    &uid,
                    &format!("{}:{}", file, index + 1),
                ));
            }
        }
    }
    Ok(())
}

fn compose_environment_key(value: &str) -> Option<String> {
    let value = value.trim_start_matches("- ").trim();
    let key = value
        .split_once('=')
        .map(|(key, _)| key)
        .or_else(|| value.split_once(':').map(|(key, _)| key))?
        .trim();
    (key.len() <= 128
        && !key.is_empty()
        && key.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        }))
    .then(|| key.into())
}

fn split_image_reference(image: &str) -> (&str, &str) {
    image
        .rsplit_once(':')
        .filter(|(repository, _)| !repository.ends_with('/'))
        .unwrap_or((image, "latest"))
}
fn rel(source: &str, relation: &str, target: &str, loc: &str) -> Relation {
    Relation {
        source: source.into(),
        relation: relation.into(),
        target: target.into(),
        origin: "MANIFEST".into(),
        confidence: 1.0,
        source_location: Some(loc.into()),
        properties: Default::default(),
    }
}
