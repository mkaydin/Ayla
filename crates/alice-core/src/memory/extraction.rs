// ── LLM knowledge extraction ───────────────────────────────
// Chunked, schema-constrained entity/fact/relation extraction.

use crate::memory::config::MemoryConfig;
use crate::memory::graph::NodeType;
use crate::EventSink;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const EXTRACTION_CHUNK_CHARS: usize = 7_500;
const OLLAMA_OOM_FALLBACK_CTX: usize = 16_384;
const MAX_EXTRACTION_CHUNKS: usize = 64;
const MAX_ENTITY_NAME_CHARS: usize = 180;

#[derive(Clone)]
pub struct Extractor {
    config: MemoryConfig,
    client: reqwest::Client,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedEntity {
    pub name: String,
    pub node_type: NodeType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedRelation {
    pub from: String,
    pub relation: String,
    pub to: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Default)]
pub struct KnowledgeExtract {
    pub entities: Vec<ExtractedEntity>,
    pub facts: Vec<(String, String, String, f32)>,
    pub relations: Vec<ExtractedRelation>,
}

impl KnowledgeExtract {
    fn merge(&mut self, other: KnowledgeExtract) {
        for entity in other.entities {
            self.upsert_entity(entity);
        }

        let mut fact_keys: HashSet<(String, String, String)> = self
            .facts
            .iter()
            .map(|(subject, predicate, object, _)| {
                (
                    subject.to_lowercase(),
                    predicate.to_lowercase(),
                    object.to_lowercase(),
                )
            })
            .collect();
        for fact in other.facts {
            let key = (
                fact.0.to_lowercase(),
                fact.1.to_lowercase(),
                fact.2.to_lowercase(),
            );
            if fact_keys.insert(key) {
                self.facts.push(fact);
            }
        }

        let mut relation_positions: HashMap<(String, String, String), usize> = self
            .relations
            .iter()
            .enumerate()
            .map(|(index, relation)| (relation_key(relation), index))
            .collect();
        for relation in other.relations {
            self.upsert_entity(ExtractedEntity {
                name: relation.from.clone(),
                node_type: NodeType::Concept,
            });
            self.upsert_entity(ExtractedEntity {
                name: relation.to.clone(),
                node_type: NodeType::Concept,
            });
            let key = relation_key(&relation);
            if let Some(index) = relation_positions.get(&key).copied() {
                self.relations[index].confidence =
                    self.relations[index].confidence.max(relation.confidence);
            } else {
                relation_positions.insert(key, self.relations.len());
                self.relations.push(relation);
            }
        }
    }

    fn upsert_entity(&mut self, entity: ExtractedEntity) {
        let key = entity.name.to_lowercase();
        if let Some(existing) = self
            .entities
            .iter_mut()
            .find(|candidate| candidate.name.to_lowercase() == key)
        {
            if is_generic_type(&existing.node_type) && !is_generic_type(&entity.node_type) {
                existing.node_type = entity.node_type;
            }
            return;
        }
        self.entities.push(entity);
    }
}

impl Extractor {
    pub fn new(config: MemoryConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    /// Extract knowledge from a short conversation turn.
    pub async fn extract_all(&self, text: &str) -> Result<KnowledgeExtract, String> {
        let chunks = extraction_chunks(text);
        let total = chunks.len();
        let mut merged = KnowledgeExtract::default();
        for (index, chunk) in chunks.iter().enumerate() {
            let extraction = self.extract_chunk(chunk, None, index + 1, total).await?;
            merged.merge(extraction);
        }
        Ok(merged)
    }

    /// Extract a document in bounded chunks, then make a focused second relation
    /// pass when the first pass leaves too many entities disconnected.
    #[cfg(test)]
    pub async fn extract_document(
        &self,
        text: &str,
        document_name: &str,
    ) -> Result<KnowledgeExtract, String> {
        self.extract_document_with_progress(text, document_name, None)
            .await
    }

    pub async fn extract_document_with_progress(
        &self,
        text: &str,
        document_name: &str,
        event_sink: Option<&dyn EventSink>,
    ) -> Result<KnowledgeExtract, String> {
        let chunks = extraction_chunks(text);
        let total = chunks.len();
        let mut merged = KnowledgeExtract::default();

        log::info!(
            "Knowledge extraction: {} chars in {} chunk(s), max_num_ctx={}",
            text.len(),
            total,
            self.config.ollama_num_ctx
        );

        for (index, chunk) in chunks.iter().enumerate() {
            emit_progress(
                event_sink,
                document_name,
                30 + (index * 25 / total.max(1)),
                &format!("extracting chunk {}/{}", index + 1, total),
            );
            let mut extraction = self
                .extract_chunk(chunk, Some(document_name), index + 1, total)
                .await?;

            let components = relation_component_count(&extraction.entities, &extraction.relations);
            if extraction.entities.len() > 1 && components > 1 {
                emit_progress(
                    event_sink,
                    document_name,
                    30 + (index * 25 / total.max(1)),
                    &format!("completing relations {}/{}", index + 1, total),
                );
                log::info!(
                    "Knowledge extraction chunk {}/{}: relation completion pass ({} disconnected components)",
                    index + 1,
                    total,
                    components,
                );
                let additional = self
                    .complete_relations(
                        chunk,
                        document_name,
                        &extraction.entities,
                        &extraction.relations,
                    )
                    .await?;
                extraction.merge(additional);
            }

            log::info!(
                "Knowledge extraction chunk {}/{}: {} entities, {} facts, {} relations",
                index + 1,
                total,
                extraction.entities.len(),
                extraction.facts.len(),
                extraction.relations.len()
            );
            merged.merge(extraction);
            emit_progress(
                event_sink,
                document_name,
                30 + ((index + 1) * 25 / total.max(1)),
                &format!("merged chunk {}/{}", index + 1, total),
            );
        }

        log::info!(
            "Knowledge extraction merged: {} entities, {} facts, {} relations",
            merged.entities.len(),
            merged.facts.len(),
            merged.relations.len()
        );
        Ok(merged)
    }

    async fn extract_chunk(
        &self,
        text: &str,
        document_name: Option<&str>,
        chunk_number: usize,
        chunk_total: usize,
    ) -> Result<KnowledgeExtract, String> {
        let source_label = document_name.unwrap_or("conversation_turn");
        let schema = knowledge_schema();
        let prompt = format!(
            "Build a dense, evidence-grounded knowledge graph from the source text below.\n\
             Source name: {source_label}\n\
             Source chunk: {chunk_number}/{chunk_total}\n\n\
             EXTRACTION PROCEDURE:\n\
             1. Read every sentence and resolve pronouns/coreferences to canonical names.\n\
             2. Extract named and important domain entities. Assign one type: Person, Project,\
                Tool, Concept, Document, or Other.\n\
             3. Extract every explicit factual relationship and strongly supported implicit\
                relationship. Do not stop after finding only the most obvious relations.\n\
             4. Include architecture and hierarchy relations, data/control flow, dependencies,\
                implementations, communication, storage, replication, coordination, causality,\
                comparison, ownership, references, and properties when supported.\n\
             5. Use short canonical entity names consistently. Relation endpoints must exactly\
                match entity names. Never use pronouns as entity names.\n\
             6. Use {source_label} as the Document entity when the text relates an entity to\
                the source itself. Never emit 'this_document'.\n\
             7. Do not invent unsupported claims. Confidence: 0.90-1.0 explicit, 0.70-0.89\
                strongly implied, below 0.70 only when useful and clearly plausible.\n\n\
             Keep the JSON compact. Return at most 60 entities, 80 facts, and 100 relations\
             for this chunk; prioritize domain-specific entities and relations over generic terms.\n\n\
             Useful relation names include IS_A, PART_OF, CONTAINS, MENTIONS, USES, USED_BY,\
             DEPENDS_ON, REQUIRES, IMPLEMENTS, EXTENDS, INTEGRATES_WITH, COMMUNICATES_WITH,\
             SENDS_TO, RECEIVES_FROM, STORES, READS_FROM, WRITES_TO, RUNS_ON, PRODUCES,\
             CONSUMES, ENABLES, PREVENTS, REPLICATES, COORDINATES_WITH, COMPARES_WITH,\
             REFERENCES, DESCRIBES, CREATED_BY, OWNS, PREFERS, WORKS_ON, and HAS_PROPERTY.\n\n\
             Return every useful relation supported by the text, not merely one relation per\
             entity. Output must match this JSON schema exactly:\n{}\n\n\
             SOURCE TEXT:\n{}",
            serde_json::to_string(&schema).unwrap_or_default(),
            text
        );
        let response = self.ask_llm(&prompt, schema.clone()).await?;
        match parse_knowledge(&response, document_name) {
            Ok(parsed) => Ok(parsed),
            Err(error) if error.starts_with("Extraction JSON:") => {
                log::warn!(
                    "Extraction chunk {}/{} returned malformed JSON; requesting repair",
                    chunk_number,
                    chunk_total
                );
                let repair_prompt = format!(
                    "Repair the malformed JSON below. Return ONLY a valid JSON object matching this schema. Do not add commentary.\n\nSCHEMA:\n{}\n\nMALFORMED OUTPUT:\n{}",
                    serde_json::to_string(&schema).unwrap_or_default(),
                    crate::memory::safe_truncate(&response, 24_000)
                );
                let repaired = self.ask_llm(&repair_prompt, schema).await?;
                parse_knowledge(&repaired, document_name)
                    .map_err(|repair_error| format!("{error}; repair failed: {repair_error}"))
            }
            Err(error) => Err(error),
        }
    }

    async fn complete_relations(
        &self,
        text: &str,
        document_name: &str,
        entities: &[ExtractedEntity],
        existing_relations: &[ExtractedRelation],
    ) -> Result<KnowledgeExtract, String> {
        let entity_names: Vec<&str> = entities.iter().map(|entity| entity.name.as_str()).collect();
        let existing: Vec<Value> = existing_relations
            .iter()
            .map(|relation| {
                json!({
                    "from": relation.from,
                    "relation": relation.relation,
                    "to": relation.to,
                })
            })
            .collect();
        let schema = relations_schema();
        let prompt = format!(
            "Perform a second-pass relationship audit for document '{document_name}'.\n\
             Inspect every sentence and recover meaningful relations missed by the first pass.\n\
             Prefer relations between the supplied canonical entities. You may introduce an\
             entity only when the text clearly names it. Resolve pronouns. Include hierarchy,\
             architecture, dependency, data flow, communication, storage, replication,\
             coordination, causality, implementation, comparison, and property relations.\n\
             Do not repeat an existing triple and do not invent facts merely to connect nodes.\n\n\
             CANONICAL ENTITIES:\n{}\n\n\
             EXISTING RELATIONS:\n{}\n\n\
             Output must match this JSON schema exactly:\n{}\n\n\
             SOURCE TEXT:\n{}",
            serde_json::to_string(&entity_names).unwrap_or_default(),
            serde_json::to_string(&existing).unwrap_or_default(),
            serde_json::to_string(&schema).unwrap_or_default(),
            text
        );
        let response = self.ask_llm(&prompt, schema).await?;
        parse_knowledge(&response, Some(document_name))
    }

    async fn ask_llm(&self, prompt: &str, format: Value) -> Result<String, String> {
        let format_bytes = serde_json::to_vec(&format)
            .map(|encoded| encoded.len())
            .unwrap_or_default();
        let input_bytes = prompt
            .len()
            .saturating_add(format_bytes)
            .saturating_add(160);
        let mut num_ctx = self.config.ollama_context_for(input_bytes, 8_192);
        let mut retried_after_oom = false;
        log::debug!(
            "Knowledge extraction request: input={} bytes, num_ctx={num_ctx}",
            input_bytes
        );

        loop {
            let body = json!({
                "model": self.config.llm_model,
                "messages": [
                    {
                        "role": "system",
                        "content": "You are a high-recall knowledge graph extraction engine. Return only schema-valid JSON grounded in the supplied text."
                    },
                    { "role": "user", "content": prompt }
                ],
                "stream": false,
                "think": false,
                "format": format.clone(),
                "options": {
                    "temperature": 0.1,
                    "seed": 42,
                    "num_ctx": num_ctx,
                    "num_predict": 8192
                }
            });

            let response = self
                .client
                .post(format!("{}/api/chat", self.config.ollama_url))
                .json(&body)
                .send()
                .await
                .map_err(|error| format!("LLM request: {}", error))?;
            let status = response.status();
            let text = response
                .text()
                .await
                .map_err(|error| format!("LLM response: {}", error))?;

            if !status.is_success() {
                if !retried_after_oom && num_ctx > OLLAMA_OOM_FALLBACK_CTX && is_ollama_oom(&text) {
                    log::warn!(
                        "Ollama exhausted GPU memory at num_ctx={num_ctx}; retrying extraction at num_ctx={OLLAMA_OOM_FALLBACK_CTX}"
                    );
                    num_ctx = OLLAMA_OOM_FALLBACK_CTX;
                    retried_after_oom = true;
                    continue;
                }
                return Err(format!(
                    "LLM HTTP {}: {}",
                    status,
                    crate::memory::safe_truncate(&text, 500)
                ));
            }

            let envelope: Value =
                serde_json::from_str(&text).map_err(|error| format!("Ollama JSON: {}", error))?;
            let content = envelope["message"]["content"]
                .as_str()
                .ok_or("Ollama response did not contain message.content")?
                .trim()
                .to_string();
            if content.is_empty() {
                return Err("Ollama returned an empty extraction".to_string());
            }
            return Ok(content);
        }
    }
}

fn is_ollama_oom(response_body: &str) -> bool {
    let lower = response_body.to_ascii_lowercase();
    lower.contains("out of memory")
        && (lower.contains("cuda") || lower.contains("gpu") || lower.contains("vram"))
}

fn emit_progress(event_sink: Option<&dyn EventSink>, source: &str, pct: usize, step: &str) {
    if let Some(events) = event_sink {
        let _ = events.emit(
            "ingest-progress",
            json!({ "step": step, "pct": pct, "source": source }),
        );
    }
}

fn extraction_chunks(text: &str) -> Vec<String> {
    let mut chunks = crate::tools::quick_extract::chunk_text(text, EXTRACTION_CHUNK_CHARS);
    if chunks.len() > MAX_EXTRACTION_CHUNKS {
        log::warn!(
            "Knowledge extraction limited from {} to {} chunks",
            chunks.len(),
            MAX_EXTRACTION_CHUNKS
        );
        chunks.truncate(MAX_EXTRACTION_CHUNKS);
    }
    chunks
}

fn knowledge_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "entities": {
                "type": "array",
                "maxItems": 60,
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "type": {
                            "type": "string",
                            "enum": ["Person", "Project", "Tool", "Concept", "Document", "Other"]
                        }
                    },
                    "required": ["name", "type"],
                    "additionalProperties": false
                }
            },
            "facts": {
                "type": "array",
                "maxItems": 80,
                "items": {
                    "type": "object",
                    "properties": {
                        "subject": { "type": "string" },
                        "predicate": { "type": "string" },
                        "object": { "type": "string" },
                        "confidence": { "type": "number" }
                    },
                    "required": ["subject", "predicate", "object", "confidence"],
                    "additionalProperties": false
                }
            },
            "relations": relation_array_schema()
        },
        "required": ["entities", "facts", "relations"],
        "additionalProperties": false
    })
}

fn relations_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "relations": relation_array_schema()
        },
        "required": ["relations"],
        "additionalProperties": false
    })
}

fn relation_array_schema() -> Value {
    json!({
        "type": "array",
        "maxItems": 100,
        "items": {
            "type": "object",
            "properties": {
                "from": { "type": "string" },
                "relation": { "type": "string" },
                "to": { "type": "string" },
                "confidence": { "type": "number" }
            },
            "required": ["from", "relation", "to", "confidence"],
            "additionalProperties": false
        }
    })
}

fn parse_knowledge(
    response: &str,
    document_name: Option<&str>,
) -> Result<KnowledgeExtract, String> {
    let cleaned = response.trim();
    let json_str = if let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}')) {
        &cleaned[start..=end]
    } else {
        return Err("Extraction response did not contain a JSON object".to_string());
    };
    let object: Value =
        serde_json::from_str(json_str).map_err(|error| format!("Extraction JSON: {}", error))?;
    let mut result = KnowledgeExtract::default();

    if let Some(entities) = object.get("entities").and_then(Value::as_array) {
        for value in entities {
            let parsed = if let Some(name) = value.as_str() {
                normalize_entity(name, document_name).map(|name| ExtractedEntity {
                    name,
                    node_type: NodeType::Concept,
                })
            } else {
                let name = value.get("name").and_then(Value::as_str).unwrap_or("");
                normalize_entity(name, document_name).map(|name| ExtractedEntity {
                    name,
                    node_type: parse_node_type(
                        value
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("Concept"),
                    ),
                })
            };
            if let Some(entity) = parsed {
                result.upsert_entity(entity);
            }
        }
    }

    if let Some(facts) = object.get("facts").and_then(Value::as_array) {
        for value in facts {
            let parsed = if let Some(array) = value.as_array() {
                if array.len() < 3 {
                    None
                } else {
                    Some((
                        array[0].as_str().unwrap_or(""),
                        array[1].as_str().unwrap_or(""),
                        array[2].as_str().unwrap_or(""),
                        array.get(3).and_then(Value::as_f64).unwrap_or(0.7) as f32,
                    ))
                }
            } else {
                Some((
                    value.get("subject").and_then(Value::as_str).unwrap_or(""),
                    value.get("predicate").and_then(Value::as_str).unwrap_or(""),
                    value.get("object").and_then(Value::as_str).unwrap_or(""),
                    value
                        .get("confidence")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.7) as f32,
                ))
            };
            if let Some((subject, predicate, object, confidence)) = parsed {
                let subject = subject.trim();
                let predicate = predicate.trim();
                let object = object.trim();
                if !subject.is_empty() && !predicate.is_empty() && !object.is_empty() {
                    result.facts.push((
                        subject.to_string(),
                        predicate.to_string(),
                        object.to_string(),
                        confidence.clamp(0.0, 1.0),
                    ));
                }
            }
        }
    }

    if let Some(relations) = object.get("relations").and_then(Value::as_array) {
        for value in relations {
            let parsed = if let Some(array) = value.as_array() {
                if array.len() < 3 {
                    None
                } else {
                    Some((
                        array[0].as_str().unwrap_or(""),
                        array[1].as_str().unwrap_or(""),
                        array[2].as_str().unwrap_or(""),
                        array.get(3).and_then(Value::as_f64).unwrap_or(0.8) as f32,
                    ))
                }
            } else {
                Some((
                    value.get("from").and_then(Value::as_str).unwrap_or(""),
                    value
                        .get("relation")
                        .or_else(|| value.get("type"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    value.get("to").and_then(Value::as_str).unwrap_or(""),
                    value
                        .get("confidence")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.8) as f32,
                ))
            };
            let Some((from, relation, to, confidence)) = parsed else {
                continue;
            };
            let Some(from) = normalize_entity(from, document_name) else {
                continue;
            };
            let Some(to) = normalize_entity(to, document_name) else {
                continue;
            };
            let relation = normalize_relation(relation);
            if from.eq_ignore_ascii_case(&to) || relation.is_empty() {
                continue;
            }
            result.merge(KnowledgeExtract {
                entities: vec![],
                facts: vec![],
                relations: vec![ExtractedRelation {
                    from,
                    relation,
                    to,
                    confidence: confidence.clamp(0.0, 1.0),
                }],
            });
        }
    }

    Ok(result)
}

fn normalize_entity(value: &str, document_name: Option<&str>) -> Option<String> {
    let mut name = value
        .trim()
        .trim_matches(|character: char| character == '"' || character == '\'' || character == '`')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(document_name) = document_name {
        let lowered = name.to_lowercase();
        if matches!(
            lowered.as_str(),
            "this_document" | "this document" | "the document" | "source document"
        ) {
            name = document_name.to_string();
        }
    }
    if name.len() < 2 || name.chars().count() > MAX_ENTITY_NAME_CHARS {
        return None;
    }
    Some(name)
}

fn normalize_relation(value: &str) -> String {
    let mut normalized = String::new();
    let mut separator = false;
    for character in value.trim().chars() {
        if character.is_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push('_');
            }
            for upper in character.to_uppercase() {
                normalized.push(upper);
            }
            separator = false;
        } else {
            separator = true;
        }
    }
    normalized
}

fn parse_node_type(value: &str) -> NodeType {
    match value.trim().to_lowercase().as_str() {
        "person" => NodeType::Person,
        "project" | "system" | "application" => NodeType::Project,
        "tool" | "technology" | "library" | "framework" | "database" => NodeType::Tool,
        "document" | "file" | "paper" => NodeType::Document,
        "other" => NodeType::Other,
        _ => NodeType::Concept,
    }
}

fn is_generic_type(node_type: &NodeType) -> bool {
    matches!(node_type, NodeType::Concept | NodeType::Other)
}

fn relation_key(relation: &ExtractedRelation) -> (String, String, String) {
    (
        relation.from.to_lowercase(),
        relation.relation.to_lowercase(),
        relation.to.to_lowercase(),
    )
}

#[cfg(test)]
fn connected_entity_count(relations: &[ExtractedRelation]) -> usize {
    relations
        .iter()
        .flat_map(|relation| [relation.from.to_lowercase(), relation.to.to_lowercase()])
        .collect::<HashSet<_>>()
        .len()
}

fn relation_component_count(
    entities: &[ExtractedEntity],
    relations: &[ExtractedRelation],
) -> usize {
    if entities.is_empty() {
        return 0;
    }
    let mut adjacency: HashMap<String, Vec<String>> = entities
        .iter()
        .map(|entity| (entity.name.to_lowercase(), Vec::new()))
        .collect();
    for relation in relations {
        let from = relation.from.to_lowercase();
        let to = relation.to.to_lowercase();
        adjacency.entry(from.clone()).or_default().push(to.clone());
        adjacency.entry(to).or_default().push(from);
    }

    let mut seen = HashSet::new();
    let mut components = 0;
    for entity in adjacency.keys() {
        if seen.contains(entity) {
            continue;
        }
        components += 1;
        let mut stack = vec![entity.clone()];
        while let Some(current) = stack.pop() {
            if !seen.insert(current.clone()) {
                continue;
            }
            if let Some(neighbors) = adjacency.get(&current) {
                stack.extend(neighbors.iter().cloned());
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_cuda_oom_responses_only() {
        assert!(is_ollama_oom(
            r#"{"error":"CUDA error: out of memory\nCUDA error"}"#
        ));
        assert!(!is_ollama_oom(
            r#"{"error":"model runner failed to start"}"#
        ));
    }

    #[test]
    fn parses_typed_entities_and_relation_objects() {
        let response = r#"{
          "entities": [
            {"name":"Raft","type":"Concept"},
            {"name":"etcd","type":"Tool"}
          ],
          "facts": [],
          "relations": [
            {"from":"etcd","relation":"implements","to":"Raft","confidence":0.96}
          ]
        }"#;
        let parsed = parse_knowledge(response, Some("distributed.md")).unwrap();
        assert_eq!(parsed.entities.len(), 2);
        assert_eq!(parsed.relations.len(), 1);
        assert_eq!(parsed.relations[0].relation, "IMPLEMENTS");
        assert_eq!(parsed.relations[0].confidence, 0.96);
    }

    #[test]
    fn adds_relation_endpoints_and_rewrites_document_reference() {
        let response = r#"{
          "entities": [],
          "facts": [],
          "relations": [
            ["this_document","describes","Consensus",0.9]
          ]
        }"#;
        let parsed = parse_knowledge(response, Some("distributed.md")).unwrap();
        assert_eq!(parsed.entities.len(), 2);
        assert_eq!(parsed.relations[0].from, "distributed.md");
        assert_eq!(parsed.relations[0].to, "Consensus");
    }

    #[test]
    fn merge_deduplicates_case_insensitively_and_keeps_confidence() {
        let mut first = KnowledgeExtract {
            entities: vec![],
            facts: vec![],
            relations: vec![ExtractedRelation {
                from: "Raft".into(),
                relation: "USES".into(),
                to: "Quorums".into(),
                confidence: 0.7,
            }],
        };
        first.merge(KnowledgeExtract {
            entities: vec![],
            facts: vec![],
            relations: vec![ExtractedRelation {
                from: "raft".into(),
                relation: "uses".into(),
                to: "quorums".into(),
                confidence: 0.95,
            }],
        });
        assert_eq!(first.relations.len(), 1);
        assert_eq!(first.relations[0].confidence, 0.95);
        assert_eq!(first.entities.len(), 2);
    }

    #[tokio::test]
    #[ignore = "requires a running Ollama instance and ALICE_EXTRACTION_FIXTURE"]
    async fn live_document_extraction_smoke() {
        let path = std::env::var("ALICE_EXTRACTION_FIXTURE")
            .expect("set ALICE_EXTRACTION_FIXTURE to a text document");
        let text = std::fs::read_to_string(&path).expect("read extraction fixture");
        let name = std::path::Path::new(&path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("fixture");
        let extraction = Extractor::new(MemoryConfig::default())
            .extract_document(&text, name)
            .await
            .expect("live extraction");
        eprintln!(
            "live extraction: {} entities, {} facts, {} relations",
            extraction.entities.len(),
            extraction.facts.len(),
            extraction.relations.len()
        );
        assert!(extraction.entities.len() >= 5);
        assert!(extraction.relations.len() >= 5);
        assert!(connected_entity_count(&extraction.relations) >= 5);
    }
}
