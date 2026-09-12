use crate::model::{
    Diagnostic, DiagnosticLevel, Entity, GenerateOptions, KnowledgePackage, Relation,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

/// Semantic-provider boundary. Ayla can implement this trait for its own model gateway.
#[allow(async_fn_in_trait)]
pub trait LlmClient {
    async fn describe(
        &self,
        model: &str,
        base_url: &url::Url,
        entity: &Entity,
    ) -> Result<SemanticResponse, String>;

    async fn describe_batch(
        &self,
        model: &str,
        base_url: &url::Url,
        entities: &[Entity],
    ) -> Result<Vec<SemanticBatchItem>, String> {
        let mut items = Vec::with_capacity(entities.len());
        for entity in entities {
            items.push(SemanticBatchItem {
                uid: entity.uid.clone(),
                interpretation: self.describe(model, base_url, entity).await?,
            });
        }
        Ok(items)
    }
}

pub struct OllamaClient {
    client: reqwest::Client,
}
impl Default for OllamaClient {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SemanticResponse {
    pub purpose: String,
    #[serde(default)]
    pub responsibilities: Vec<String>,
    #[serde(default)]
    pub concepts: Vec<String>,
    /// Relationships between domain concepts explicitly supported by the supplied unit.
    #[serde(default)]
    pub relationships: Vec<SemanticConceptRelation>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SemanticConceptRelation {
    pub source: String,
    pub relation: String,
    pub target: String,
    #[serde(default = "default_semantic_confidence")]
    pub confidence: f32,
}

fn default_semantic_confidence() -> f32 {
    0.75
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SemanticBatchItem {
    pub uid: String,
    #[serde(flatten)]
    pub interpretation: SemanticResponse,
}

fn semantic_response_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "purpose": { "type": "string" },
            "responsibilities": { "type": "array", "items": { "type": "string" } },
            "concepts": { "type": "array", "items": { "type": "string" } },
            "relationships": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "source": { "type": "string" },
                        "relation": { "type": "string" },
                        "target": { "type": "string" },
                        "confidence": { "type": "number" }
                    },
                    "required": ["source", "relation", "target"]
                }
            }
        },
        "required": ["purpose"]
    })
}

fn semantic_batch_schema() -> Value {
    let mut item = semantic_response_schema();
    if let Some(properties) = item.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert("uid".into(), serde_json::json!({ "type": "string" }));
    }
    if let Some(required) = item.get_mut("required").and_then(Value::as_array_mut) {
        required.insert(0, Value::String("uid".into()));
    }
    serde_json::json!({
        "type": "object",
        "properties": {
            "items": { "type": "array", "items": item }
        },
        "required": ["items"]
    })
}

fn strip_json_fence(content: &str) -> &str {
    let trimmed = content.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let content = rest
        .find('\n')
        .map(|newline| &rest[newline + 1..])
        .unwrap_or(rest);
    content.strip_suffix("```").unwrap_or(content).trim()
}

fn parse_semantic_response(content: &str) -> Result<SemanticResponse, String> {
    serde_json::from_str(strip_json_fence(content))
        .map_err(|error| format!("invalid structured Ollama response: {error}"))
}

fn parse_semantic_batch_response(content: &str) -> Result<Vec<SemanticBatchItem>, String> {
    let value: Value = serde_json::from_str(strip_json_fence(content))
        .map_err(|error| format!("invalid structured Ollama batch response: {error}"))?;
    let values = semantic_batch_values(value).ok_or_else(|| {
        "invalid structured Ollama batch response: expected an items array".to_string()
    })?;
    values
        .into_iter()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| format!("invalid structured Ollama batch item: {error}"))
        })
        .collect()
}

fn semantic_batch_values(value: Value) -> Option<Vec<Value>> {
    match value {
        Value::Array(items) => Some(items),
        Value::Object(mut object) => {
            for key in ["items", "results", "analyses", "responses", "entities"] {
                if let Some(value) = object.remove(key) {
                    return semantic_batch_values(value);
                }
            }
            if object.contains_key("uid") && object.contains_key("purpose") {
                return Some(vec![Value::Object(object)]);
            }
            let mut keyed = Vec::new();
            for (uid, value) in object {
                let Value::Object(mut item) = value else {
                    continue;
                };
                if !item.contains_key("purpose") {
                    continue;
                }
                item.entry("uid").or_insert(Value::String(uid));
                keyed.push(Value::Object(item));
            }
            (!keyed.is_empty()).then_some(keyed)
        }
        _ => None,
    }
}

fn retain_expected_items(items: &mut Vec<SemanticBatchItem>, entities: &[Entity]) {
    let expected = entities
        .iter()
        .map(|entity| entity.uid.as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    items.retain(|item| expected.contains(item.uid.as_str()) && seen.insert(item.uid.clone()));
}

impl LlmClient for OllamaClient {
    async fn describe(
        &self,
        model: &str,
        base_url: &url::Url,
        entity: &Entity,
    ) -> Result<SemanticResponse, String> {
        let endpoint = base_url.join("api/chat").map_err(|e| e.to_string())?;
        let prompt = format!("Return compact JSON only with purpose (at most 60 words), responsibilities (at most 4 short strings), concepts (at most 4 strings), and relationships (at most 4 objects with source, relation, target, confidence). Describe this repository knowledge unit using only supplied facts. Do not invent APIs or dependencies. Concepts must be domain or architectural concepts; exclude language primitives, container types, property accessors, default values, and generic syntax terms. Relationship must be one of RELATED_TO, PART_OF, ENABLES, USES, IMPLEMENTS, MITIGATES, or SUPPORTS. Entity type: {}. Name: {}. Description: {}. Location: {}. Source material:\n```text\n{}\n```", entity.label, entity.name, entity.description.as_deref().unwrap_or("not available"), entity.source_location.as_deref().unwrap_or("unknown"), entity.source_excerpt.as_deref().unwrap_or("not available"));
        let body = serde_json::json!({"model": model, "stream": false, "format": semantic_response_schema(), "messages": [{"role": "user", "content": prompt}]});
        let response: OllamaChatResponse = self
            .client
            .post(endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        parse_semantic_response(&response.message.content)
    }

    async fn describe_batch(
        &self,
        model: &str,
        base_url: &url::Url,
        entities: &[Entity],
    ) -> Result<Vec<SemanticBatchItem>, String> {
        let endpoint = base_url.join("api/chat").map_err(|e| e.to_string())?;
        let facts = entities
            .iter()
            .map(|entity| {
                serde_json::json!({
                    "uid": entity.uid,
                    "type": entity.label,
                    "name": entity.name,
                    "description": entity.description,
                    "location": entity.source_location,
                    "properties": entity.properties,
                    "source": entity.source_excerpt,
                })
            })
            .collect::<Vec<_>>();
        let prompt = format!("Interpret every supplied repository knowledge unit. Return compact JSON as {{\"items\":[{{\"uid\":\"exact supplied uid\",\"purpose\":\"at most 60 words\",\"responsibilities\":[\"at most 4 short items\"],\"concepts\":[\"at most 4 domain concepts\"],\"relationships\":[{{\"source\":\"concept name\",\"relation\":\"RELATED_TO|PART_OF|ENABLES|USES|IMPLEMENTS|MITIGATES|SUPPORTS\",\"target\":\"concept name\",\"confidence\":0.75}}]}}]}}. Include exactly one item per input UID and at most 4 relationships per item. Use only supplied facts. Exclude primitives, container types, property accessors, defaults, and syntax terms. Only include a relationship when the source material directly supports it. Input: {}", serde_json::to_string(&facts).map_err(|e| e.to_string())?);
        let body = serde_json::json!({"model": model, "stream": false, "format": semantic_batch_schema(), "messages": [{"role": "user", "content": prompt}]});
        let response: OllamaChatResponse = self
            .client
            .post(endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let batch_error;
        let mut items = match parse_semantic_batch_response(&response.message.content) {
            Ok(items) => {
                batch_error = None;
                items
            }
            Err(error) => {
                batch_error = Some(error);
                Vec::new()
            }
        };
        retain_expected_items(&mut items, entities);
        let mut seen = items
            .iter()
            .map(|item| item.uid.clone())
            .collect::<HashSet<_>>();

        let mut fallback_errors = Vec::new();
        for entity in entities {
            if seen.contains(&entity.uid) {
                continue;
            }
            match self.describe(model, base_url, entity).await {
                Ok(interpretation) => {
                    seen.insert(entity.uid.clone());
                    items.push(SemanticBatchItem {
                        uid: entity.uid.clone(),
                        interpretation,
                    });
                }
                Err(error) => fallback_errors.push(format!("{}: {error}", entity.uid)),
            }
        }
        if items.is_empty() {
            let batch_error = batch_error
                .unwrap_or_else(|| "batch response contained no exact supplied UIDs".to_string());
            return Err(format!(
                "{batch_error}; per-item fallback failed: {}",
                fallback_errors.join("; ")
            ));
        }
        Ok(items)
    }
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: OllamaMessage,
}
#[derive(Deserialize)]
struct OllamaMessage {
    content: String,
}

impl KnowledgePackage {
    pub(crate) async fn apply_semantics<C: LlmClient>(
        &mut self,
        llm: &C,
        options: &GenerateOptions,
    ) {
        if !options.semantic_analysis || matches!(options.analysis_mode, crate::AnalysisMode::Fast)
        {
            return;
        }
        let candidates: Vec<_> = self
            .entities
            .iter()
            .filter(|entity| {
                matches!(
                    entity.label.as_str(),
                    "Repository"
                        | "SystemOverview"
                        | "Documentation"
                        | "Struct"
                        | "Trait"
                        | "Enum"
                        | "Class"
                        | "Function"
                        | "Method"
                        | "Module"
                )
            })
            .cloned()
            .collect();
        let mut context: Vec<_> = candidates
            .iter()
            .filter(|entity| matches!(entity.label.as_str(), "Repository" | "SystemOverview"))
            .cloned()
            .collect();
        let mut documentation: Vec<_> = candidates
            .iter()
            .filter(|entity| entity.label == "Documentation")
            .cloned()
            .collect();
        documentation.sort_by_key(document_semantic_priority);
        if let Some(limit) = options.semantic_document_limit {
            documentation.truncate(limit);
        }
        context.extend(documentation);
        let mut code: Vec<_> = candidates
            .into_iter()
            .filter(|entity| {
                !matches!(
                    entity.label.as_str(),
                    "Repository" | "SystemOverview" | "Documentation"
                )
            })
            .collect();
        code.sort_by_key(semantic_priority);
        if let Some(limit) = options.semantic_symbol_limit {
            code.truncate(limit);
        }
        context.extend(code);
        let candidates = context;
        let total = candidates.len();
        let batches = candidates.chunks(8).collect::<Vec<_>>();
        eprintln!(
            "[kb-ready-code-doc] semantic: interpreting {total} prioritized units with {}",
            options.model
        );
        for (batch_index, batch) in batches.iter().enumerate() {
            eprintln!(
                "[kb-ready-code-doc] semantic batch {}/{}: {} units",
                batch_index + 1,
                batches.len(),
                batch.len()
            );
            match llm
                .describe_batch(&options.model, &options.ollama_base_url, batch)
                .await
            {
                Ok(items) => {
                    let returned = items.len();
                    let returned_uids = items
                        .iter()
                        .map(|item| item.uid.as_str())
                        .collect::<HashSet<_>>();
                    for entity in *batch {
                        if !returned_uids.contains(entity.uid.as_str()) {
                            self.diagnostics.push(Diagnostic {
                                level: DiagnosticLevel::Warning,
                                message: format!(
                                    "Semantic analysis returned no valid result for {}",
                                    entity.name
                                ),
                            });
                        }
                    }
                    for item in items {
                        let Some(entity) = batch.iter().find(|entity| entity.uid == item.uid)
                        else {
                            continue;
                        };
                        let semantic = item.interpretation;
                        if let Some(stored) = self
                            .entities
                            .iter_mut()
                            .find(|value| value.uid == entity.uid)
                        {
                            let responsibilities = semantic
                                .responsibilities
                                .iter()
                                .filter(|value| !value.trim().is_empty())
                                .cloned()
                                .collect::<Vec<_>>();
                            stored.description = Some(semantic.purpose.clone());
                            stored
                                .properties
                                .insert("semanticInterpreted".into(), serde_json::json!(true));
                            stored.embedding_text = Some(if responsibilities.is_empty() {
                                semantic.purpose.clone()
                            } else {
                                format!(
                                    "{} Responsibilities: {}.",
                                    semantic.purpose,
                                    responsibilities.join("; ")
                                )
                            });
                        }
                        for concept in semantic
                            .concepts
                            .into_iter()
                            .filter(|value| is_meaningful_concept(value))
                            .take(5)
                        {
                            let confidence = concept_confidence(
                                &concept,
                                entity.source_excerpt.as_deref().unwrap_or(""),
                            );
                            if let Some(dependency_uid) = declared_dependency_uid(self, &concept) {
                                self.add_relation(Relation {
                                    source: entity.uid.clone(),
                                    relation: "MENTIONS_DEPENDENCY".into(),
                                    target: dependency_uid,
                                    origin: "LLM".into(),
                                    confidence,
                                    source_location: entity.source_location.clone(),
                                    properties: Default::default(),
                                });
                                continue;
                            }
                            if let Some(uid) = upsert_concept(
                                self,
                                &self.project_name.clone(),
                                &concept,
                                entity.source_location.as_deref(),
                                confidence,
                            ) {
                                self.add_relation(Relation {
                                    source: entity.uid.clone(),
                                    relation: "PART_OF_SUBSYSTEM".into(),
                                    target: uid,
                                    origin: "LLM".into(),
                                    confidence,
                                    source_location: entity.source_location.clone(),
                                    properties: Default::default(),
                                });
                            }
                        }
                        for relationship in semantic.relationships.into_iter().take(8) {
                            if !is_meaningful_concept(&relationship.source)
                                || !is_meaningful_concept(&relationship.target)
                            {
                                continue;
                            }
                            if declared_dependency_uid(self, &relationship.source).is_some()
                                || declared_dependency_uid(self, &relationship.target).is_some()
                            {
                                continue;
                            }
                            let Some(relation) =
                                normalized_concept_relation(&relationship.relation)
                            else {
                                continue;
                            };
                            let confidence = relationship.confidence.clamp(0.5, 0.9);
                            let project_name = self.project_name.clone();
                            let Some(source_uid) = upsert_concept(
                                self,
                                &project_name,
                                &relationship.source,
                                entity.source_location.as_deref(),
                                confidence,
                            ) else {
                                continue;
                            };
                            let Some(target_uid) = upsert_concept(
                                self,
                                &project_name,
                                &relationship.target,
                                entity.source_location.as_deref(),
                                confidence,
                            ) else {
                                continue;
                            };
                            self.add_relation(Relation {
                                source: source_uid,
                                relation: relation.into(),
                                target: target_uid,
                                origin: "LLM".into(),
                                confidence,
                                source_location: entity.source_location.clone(),
                                properties: [("evidenceUid".into(), serde_json::json!(entity.uid))]
                                    .into_iter()
                                    .collect(),
                            });
                        }
                    }
                    eprintln!(
                        "[kb-ready-code-doc] semantic batch {}/{}: complete ({returned}/{})",
                        batch_index + 1,
                        batches.len(),
                        batch.len()
                    );
                }
                Err(error) => {
                    eprintln!(
                        "[kb-ready-code-doc] semantic batch {}/{}: failed ({error})",
                        batch_index + 1,
                        batches.len()
                    );
                    for entity in *batch {
                        self.diagnostics.push(Diagnostic {
                            level: DiagnosticLevel::Warning,
                            message: format!(
                                "Semantic analysis skipped for {}: {error}",
                                entity.name
                            ),
                        });
                    }
                }
            }
        }
        self.deduplicate();
    }
}

fn is_meaningful_concept(value: &str) -> bool {
    let normalized = crate::model::safe_file_stem(value);
    !normalized.is_empty()
        && !matches!(
            normalized.as_str(),
            "str"
                | "string"
                | "int"
                | "integer"
                | "float"
                | "bool"
                | "boolean"
                | "none"
                | "null"
                | "list"
                | "dict"
                | "dictionary"
                | "tuple"
                | "set"
                | "optional"
                | "union"
                | "any"
                | "default"
                | "default-value"
                | "property"
                | "property-accessor"
                | "getter"
                | "setter"
                | "choices"
                | "field"
                | "variable"
                | "parameter"
                | "return-value"
                | "type-annotation"
                | "user"
                | "software"
                | "license"
                | "copyright"
                | "warranty"
        )
}

fn declared_dependency_uid(package: &KnowledgePackage, concept: &str) -> Option<String> {
    let concept = canonical_concept_slug(concept);
    package
        .entities
        .iter()
        .find(|entity| {
            entity.label == "DependencyPackage" && canonical_concept_slug(&entity.name) == concept
        })
        .map(|entity| entity.uid.clone())
}

fn concept_confidence(concept: &str, source: &str) -> f32 {
    let needle = concept.replace(['-', '_'], " ").to_ascii_lowercase();
    let haystack = source.replace(['-', '_'], " ").to_ascii_lowercase();
    if haystack.contains(&needle) {
        0.90
    } else if concept.contains('-') || concept.split_whitespace().count() > 1 {
        0.75
    } else {
        0.50
    }
}

fn upsert_concept(
    package: &mut KnowledgePackage,
    project: &str,
    concept: &str,
    source_location: Option<&str>,
    confidence: f32,
) -> Option<String> {
    let slug = canonical_concept_slug(concept);
    if slug.is_empty() {
        return None;
    }
    let uid = format!(
        "concept:{}.domain.{slug}",
        crate::model::safe_file_stem(project)
    );
    if let Some(existing) = package.entities.iter_mut().find(|entity| entity.uid == uid) {
        existing.confidence = existing.confidence.max(confidence);
        let aliases = existing
            .properties
            .entry("aliases".into())
            .or_insert_with(|| serde_json::json!([]));
        if let Some(values) = aliases.as_array_mut() {
            if !values.iter().any(|value| value.as_str() == Some(concept)) {
                values.push(serde_json::json!(concept));
            }
        }
        if let Some(location) = source_location {
            let locations = existing
                .properties
                .entry("sourceLocations".into())
                .or_insert_with(|| serde_json::json!([]));
            if let Some(values) = locations.as_array_mut() {
                if !values.iter().any(|value| value.as_str() == Some(location)) {
                    values.push(serde_json::json!(location));
                }
            }
        }
        return Some(uid);
    }
    let source_locations = source_location
        .map(|location| vec![location])
        .unwrap_or_default();
    package.add_entity(Entity {
        uid: uid.clone(),
        label: "Concept".into(),
        name: concept.into(),
        description: Some(concept.into()),
        source_location: None,
        origin: "LLM".into(),
        confidence,
        embedding_text: Some(concept.into()),
        properties: [
            ("aliases".into(), serde_json::json!([concept])),
            ("namespace".into(), serde_json::json!(project)),
            (
                "sourceLocations".into(),
                serde_json::json!(source_locations),
            ),
        ]
        .into_iter()
        .collect(),
        source_excerpt: None,
    });
    Some(uid)
}

fn canonical_concept_slug(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalized_concept_relation(value: &str) -> Option<&'static str> {
    match value
        .trim()
        .to_ascii_uppercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "RELATED_TO" => Some("RELATED_TO"),
        "PART_OF" | "BELONGS_TO" => Some("PART_OF"),
        "ENABLES" => Some("ENABLES"),
        "USES" => Some("USES"),
        "IMPLEMENTS" => Some("IMPLEMENTS"),
        "MITIGATES" => Some("MITIGATES"),
        "SUPPORTS" => Some("SUPPORTS"),
        _ => None,
    }
}

fn semantic_priority(entity: &Entity) -> (u8, String) {
    let priority = match entity.label.as_str() {
        "Repository" | "Documentation" => 0,
        "Class" | "Struct" | "Trait" | "Module" => 1,
        "Function" => 2,
        "Method" => 3,
        _ => 4,
    };
    (priority, entity.source_location.clone().unwrap_or_default())
}

fn document_semantic_priority(entity: &Entity) -> (u8, usize, String) {
    let location = entity.source_location.as_deref().unwrap_or("");
    let name = location
        .rsplit('/')
        .next()
        .unwrap_or(location)
        .to_ascii_lowercase();
    let rank = if !location.contains('/') && name.starts_with("readme") {
        0
    } else if name.starts_with("readme") {
        1
    } else if ["architecture", "overview", "design", "getting-started"]
        .iter()
        .any(|word| name.contains(word))
    {
        2
    } else if name.starts_with("license") || name.starts_with("requirements") {
        8
    } else if location.contains("/references/") || location.contains("/generated/") {
        9
    } else {
        3
    };
    (rank, location.matches('/').count(), location.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(uid: &str) -> Entity {
        Entity {
            uid: uid.into(),
            label: "Documentation".into(),
            name: uid.into(),
            description: None,
            source_location: None,
            origin: "TEST".into(),
            confidence: 1.0,
            embedding_text: None,
            properties: Default::default(),
            source_excerpt: None,
        }
    }

    #[test]
    fn batch_parser_accepts_wrapped_array_and_keyed_safe_alternates() {
        let wrapped = r#"{"results":[{"uid":"one","purpose":"First"}]}"#;
        let parsed = parse_semantic_batch_response(wrapped).unwrap();
        assert_eq!(parsed[0].uid, "one");
        assert_eq!(parsed[0].interpretation.purpose, "First");

        let keyed = r#"{"two":{"purpose":"Second","concepts":["RAG"]}}"#;
        let parsed = parse_semantic_batch_response(keyed).unwrap();
        assert_eq!(parsed[0].uid, "two");
        assert_eq!(parsed[0].interpretation.concepts, ["RAG"]);
    }

    #[test]
    fn batch_results_are_limited_to_unique_exact_input_uids() {
        let mut parsed = parse_semantic_batch_response(
            r#"[{"uid":"one","purpose":"First"},{"uid":"unknown","purpose":"Injected"},{"uid":"one","purpose":"Duplicate"}]"#,
        )
        .unwrap();
        retain_expected_items(&mut parsed, &[entity("one"), entity("two")]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].uid, "one");
        assert_eq!(parsed[0].interpretation.purpose, "First");
    }
}
