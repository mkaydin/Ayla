//! Optional semantic assembly for OCR knowledge packages through a loopback Ollama endpoint.

use crate::output::PageResult;
use crate::types::{OcrKnowledgeEntity, OcrKnowledgePackage, OcrKnowledgeRelation};
use anyhow::{Context, Result};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::Duration;

const MAX_PAGES_PER_REQUEST: usize = 6;
const MAX_INPUT_BYTES: usize = 48 * 1024;
const MAX_SEMANTIC_ENTITIES: usize = 4_000;
const MAX_ITEMS_PER_RESPONSE: usize = 256;
const MAX_ASSEMBLY_REQUESTS: usize = 64;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SemanticAssembly {
    document_summary: String,
    page_summaries: Vec<PageSummary>,
    concepts: Vec<Concept>,
    facts: Vec<Fact>,
    relationships: Vec<Relationship>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PageSummary {
    page: u16,
    summary: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Concept {
    id: String,
    name: String,
    description: String,
    pages: Vec<u16>,
    confidence: f32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Fact {
    statement: String,
    pages: Vec<u16>,
    concept_ids: Vec<String>,
    confidence: f32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Relationship {
    source_id: String,
    relation: String,
    target_id: String,
    pages: Vec<u16>,
    confidence: f32,
}

pub(crate) struct SemanticAssembler {
    client: reqwest::Client,
    endpoint: Url,
    model: String,
    num_ctx: u32,
}

impl SemanticAssembler {
    pub(crate) fn new(
        ollama_url: &str,
        model: &str,
        num_ctx: u32,
        timeout_secs: u64,
        allow_cloud: bool,
    ) -> Result<Self> {
        let model = model.trim();
        if model.is_empty() {
            anyhow::bail!("Semantic OCR assembly requires a model name");
        }
        if model.to_ascii_lowercase().contains(":cloud") && !allow_cloud {
            anyhow::bail!(
                "OCR cloud semantic assembly requires AYLA_OCR_ASSEMBLY_ALLOW_CLOUD=true"
            );
        }
        let mut endpoint =
            Url::parse(ollama_url.trim()).context("OCR semantic assembly Ollama URL is invalid")?;
        if !matches!(endpoint.scheme(), "http" | "https") {
            anyhow::bail!("OCR semantic assembly only supports HTTP(S) Ollama endpoints");
        }
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            anyhow::bail!("OCR semantic assembly URL must not contain credentials");
        }
        let host = endpoint.host_str().unwrap_or_default();
        if !matches!(host, "localhost" | "127.0.0.1" | "::1") {
            anyhow::bail!(
                "OCR semantic assembly requires a loopback Ollama endpoint; configure localhost, 127.0.0.1, or ::1"
            );
        }
        endpoint.set_path("/api/chat");
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        let timeout = if timeout_secs == 0 {
            Duration::from_secs(180)
        } else {
            Duration::from_secs(timeout_secs)
        };
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(timeout)
            .build()
            .context("Failed to build OCR semantic assembly HTTP client")?;
        Ok(Self {
            client,
            endpoint,
            model: model.into(),
            num_ctx,
        })
    }

    pub(crate) async fn enrich(
        &self,
        pages: &[PageResult],
        package: &mut OcrKnowledgePackage,
    ) -> Result<()> {
        let recognized_pages = pages
            .iter()
            .map(|page| page.page_num)
            .collect::<HashSet<_>>();
        let mut chunks = page_chunks(pages);
        if chunks.len() > MAX_ASSEMBLY_REQUESTS {
            package.warnings.push(format!(
                "Semantic assembly was limited to the first {MAX_ASSEMBLY_REQUESTS} bounded page groups; all OCR evidence remains available"
            ));
            chunks.truncate(MAX_ASSEMBLY_REQUESTS);
        }
        let mut assemblies = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            assemblies.push(self.request_chunk(&chunk).await?);
        }
        merge_assemblies(package, assemblies, &recognized_pages);
        Ok(())
    }

    async fn request_chunk(&self, input: &str) -> Result<SemanticAssembly> {
        let prompt = format!(
            "Analyze the page-cited OCR evidence below. Return strict JSON with this exact top-level shape:\n\
             {{\"document_summary\":\"...\",\"page_summaries\":[{{\"page\":1,\"summary\":\"...\"}}],\
             \"concepts\":[{{\"id\":\"c1\",\"name\":\"...\",\"description\":\"...\",\"pages\":[1],\"confidence\":0.9}}],\
             \"facts\":[{{\"statement\":\"...\",\"pages\":[1],\"concept_ids\":[\"c1\"],\"confidence\":0.9}}],\
             \"relationships\":[{{\"source_id\":\"c1\",\"relation\":\"RELATED_TO\",\"target_id\":\"c2\",\"pages\":[1,2],\"confidence\":0.8}}]}}.\n\
             Only state information supported by the supplied pages. Preserve page citations. Find useful cross-page relationships.\n\n{input}"
        );
        let response = self
            .client
            .post(self.endpoint.clone())
            .json(&json!({
                "model": self.model,
                "stream": false,
                "format": "json",
                "messages": [{"role": "user", "content": prompt}],
                "options": {
                    "temperature": 0.0,
                    "num_ctx": self.num_ctx,
                    "num_predict": 4096
                },
                "keep_alive": "30m"
            }))
            .send()
            .await
            .with_context(|| format!("Failed to reach Ollama at {}", self.endpoint))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read OCR semantic assembly response")?;
        if !status.is_success() {
            anyhow::bail!(
                "Ollama semantic assembly HTTP {status}: {}",
                body.chars().take(500).collect::<String>()
            );
        }
        let envelope: Value =
            serde_json::from_str(&body).context("Invalid Ollama response JSON")?;
        let content = envelope
            .pointer("/message/content")
            .and_then(Value::as_str)
            .context("Ollama semantic response did not include message.content")?;
        let content = strip_json_fence(content);
        serde_json::from_str(content).context("Model returned invalid semantic assembly JSON")
    }
}

fn page_chunks(pages: &[PageResult]) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut page_count = 0usize;
    for page in pages {
        let header = format!("\n--- PAGE {} ---\n", page.page_num);
        let available = MAX_INPUT_BYTES.saturating_sub(current.len() + header.len());
        let body = truncate_utf8(page.text.trim(), available.max(1));
        if page_count >= MAX_PAGES_PER_REQUEST
            || (!current.is_empty() && current.len() + header.len() + body.len() > MAX_INPUT_BYTES)
        {
            chunks.push(std::mem::take(&mut current));
            page_count = 0;
        }
        current.push_str(&header);
        let available = MAX_INPUT_BYTES.saturating_sub(current.len());
        current.push_str(truncate_utf8(page.text.trim(), available));
        page_count += 1;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn merge_assemblies(
    package: &mut OcrKnowledgePackage,
    assemblies: Vec<SemanticAssembly>,
    recognized_pages: &HashSet<u16>,
) {
    let document_uid = package.document_uid.clone();
    let mut concept_uids = HashMap::<String, String>::new();
    let mut known_entities = package
        .entities
        .iter()
        .map(|entity| entity.uid.clone())
        .collect::<HashSet<_>>();
    let mut summaries = Vec::new();
    let mut markdown = String::from("\n## Semantic Assembly\n\n");

    for assembly in assemblies {
        let summary = clean_text(&assembly.document_summary, 2_048);
        if !summary.is_empty() {
            summaries.push(summary.clone());
            markdown.push_str(&format!("- {}\n", markdown_inline(&summary)));
        }
        for page_summary in assembly
            .page_summaries
            .into_iter()
            .take(MAX_ITEMS_PER_RESPONSE)
        {
            if !recognized_pages.contains(&page_summary.page) {
                continue;
            }
            let summary = clean_text(&page_summary.summary, 1_024);
            if summary.is_empty() {
                continue;
            }
            let page_uid = format!("{document_uid}:page:{}", page_summary.page);
            if let Some(page) = package
                .entities
                .iter_mut()
                .find(|entity| entity.uid == page_uid)
            {
                page.properties
                    .insert("semantic_summary".into(), json!(summary));
            }
            markdown.push_str(&format!(
                "\n### Page {} summary\n\n{}\n",
                page_summary.page,
                markdown_inline(&summary)
            ));
        }

        let mut local_ids = HashMap::<String, String>::new();
        for concept in assembly.concepts.into_iter().take(MAX_ITEMS_PER_RESPONSE) {
            if package.entities.len() >= MAX_SEMANTIC_ENTITIES {
                break;
            }
            let name = clean_text(&concept.name, 512);
            if name.is_empty() {
                continue;
            }
            let canonical = name.to_ascii_lowercase();
            let uid = concept_uids
                .entry(canonical)
                .or_insert_with(|| stable_uid("ocr-concept", &name))
                .clone();
            local_ids.insert(concept.id.clone(), uid.clone());
            local_ids.insert(name.clone(), uid.clone());
            if known_entities.insert(uid.clone()) {
                let pages = valid_pages(concept.pages, recognized_pages);
                let location = citation(&package.source_name, &pages);
                package.entities.push(OcrKnowledgeEntity {
                    uid: uid.clone(),
                    label: "Concept".into(),
                    name: name.clone(),
                    description: clean_text(&concept.description, 1_024),
                    source_location: location.clone(),
                    confidence: confidence(concept.confidence),
                    properties: Map::from_iter([("pages".into(), json!(pages))]),
                });
                package.relations.push(relation(
                    &uid,
                    "DESCRIBED_IN",
                    &document_uid,
                    &location,
                    confidence(concept.confidence),
                ));
                markdown.push_str(&format!(
                    "\n- Concept **{}** — {} ({})",
                    markdown_inline(&name),
                    markdown_inline(&clean_text(&concept.description, 1_024)),
                    markdown_inline(&location)
                ));
            }
        }

        for fact in assembly.facts.into_iter().take(MAX_ITEMS_PER_RESPONSE) {
            if package.entities.len() >= MAX_SEMANTIC_ENTITIES {
                break;
            }
            let statement = clean_text(&fact.statement, 1_024);
            if statement.is_empty() {
                continue;
            }
            let pages = valid_pages(fact.pages, recognized_pages);
            let location = citation(&package.source_name, &pages);
            let uid = stable_uid("ocr-fact", &format!("{statement}|{location}"));
            if known_entities.insert(uid.clone()) {
                package.entities.push(OcrKnowledgeEntity {
                    uid: uid.clone(),
                    label: "Fact".into(),
                    name: clean_text(&statement, 512),
                    description: statement.clone(),
                    source_location: location.clone(),
                    confidence: confidence(fact.confidence),
                    properties: Map::from_iter([("pages".into(), json!(pages))]),
                });
                package.relations.push(relation(
                    &uid,
                    "SUPPORTED_BY",
                    &document_uid,
                    &location,
                    confidence(fact.confidence),
                ));
            }
            for concept_id in fact.concept_ids.iter().take(16) {
                if let Some(concept_uid) = local_ids.get(concept_id) {
                    package.relations.push(relation(
                        &uid,
                        "ABOUT",
                        concept_uid,
                        &location,
                        confidence(fact.confidence),
                    ));
                }
            }
            markdown.push_str(&format!(
                "\n- Fact: {} ({})",
                markdown_inline(&statement),
                markdown_inline(&location)
            ));
        }

        for relationship in assembly
            .relationships
            .into_iter()
            .take(MAX_ITEMS_PER_RESPONSE)
        {
            let Some(source) = local_ids.get(&relationship.source_id) else {
                continue;
            };
            let Some(target) = local_ids.get(&relationship.target_id) else {
                continue;
            };
            let pages = valid_pages(relationship.pages, recognized_pages);
            let location = citation(&package.source_name, &pages);
            package.relations.push(relation(
                source,
                &safe_relation(&relationship.relation),
                target,
                &location,
                confidence(relationship.confidence),
            ));
        }
    }

    if package.entities.len() >= MAX_SEMANTIC_ENTITIES {
        package.warnings.push(format!(
            "Semantic graph output was limited to {MAX_SEMANTIC_ENTITIES} total entities"
        ));
    }
    if !summaries.is_empty() {
        if let Some(document) = package
            .entities
            .iter_mut()
            .find(|entity| entity.uid == document_uid)
        {
            document
                .properties
                .insert("semantic_summary".into(), json!(summaries.join(" ")));
        }
    }
    package.knowledge_base_markdown.push_str(&markdown);
    package.knowledge_base_markdown.push('\n');
}

fn valid_pages(pages: Vec<u16>, recognized: &HashSet<u16>) -> Vec<u16> {
    let mut pages = pages
        .into_iter()
        .filter(|page| recognized.contains(page))
        .collect::<Vec<_>>();
    pages.sort_unstable();
    pages.dedup();
    pages.truncate(32);
    pages
}

fn citation(source: &str, pages: &[u16]) -> String {
    match pages {
        [] => source.to_string(),
        [page] => format!("{source}#page={page}"),
        _ => format!(
            "{}#pages={}",
            source,
            pages
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn relation(
    source: &str,
    relation_name: &str,
    target: &str,
    source_location: &str,
    confidence: f32,
) -> OcrKnowledgeRelation {
    OcrKnowledgeRelation {
        source: source.into(),
        relation: relation_name.into(),
        target: target.into(),
        source_location: source_location.into(),
        confidence,
        properties: Map::new(),
    }
}

fn safe_relation(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() || normalized.len() > 64 {
        "RELATED_TO".into()
    } else {
        normalized.into()
    }
}

fn stable_uid(namespace: &str, value: &str) -> String {
    let digest = format!("{:x}", Sha256::digest(value.as_bytes()));
    format!("{namespace}:{}", &digest[..24])
}

fn confidence(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.5
    }
}

fn clean_text(value: &str, max_bytes: usize) -> String {
    truncate_utf8(value.trim(), max_bytes)
        .replace(['\0', '\r'], "")
        .trim()
        .to_string()
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn strip_json_fence(value: &str) -> &str {
    let trimmed = value.trim();
    let Some(without_open) = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
    else {
        return trimmed;
    };
    without_open
        .strip_suffix("```")
        .unwrap_or(without_open)
        .trim()
}

fn markdown_inline(value: &str) -> String {
    value.replace(['\r', '\n'], " ").replace('`', "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_loopback_and_explicit_cloud_consent() {
        assert!(
            SemanticAssembler::new("https://example.com", "qwen3:8b", 8192, 10, false).is_err()
        );
        assert!(
            SemanticAssembler::new("http://127.0.0.1:11434", "qwen3:cloud", 8192, 10, false)
                .is_err()
        );
        assert!(
            SemanticAssembler::new("http://127.0.0.1:11434", "qwen3:cloud", 8192, 10, true).is_ok()
        );
        assert!(
            SemanticAssembler::new("http://127.0.0.1:11434", "qwen3:8b", 8192, 10, false).is_ok()
        );
    }

    #[test]
    fn chunks_are_bounded_and_keep_page_citations() {
        let pages = (1..=9)
            .map(|page| PageResult {
                page_num: page,
                text: "x".repeat(20_000),
                image_path: String::new(),
            })
            .collect::<Vec<_>>();
        let chunks = page_chunks(&pages);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|chunk| chunk.len() <= MAX_INPUT_BYTES));
        assert!(chunks.first().unwrap().contains("--- PAGE 1 ---"));
    }

    #[test]
    fn relation_names_are_graph_safe() {
        assert_eq!(safe_relation("causes / supports"), "CAUSES___SUPPORTS");
        assert_eq!(safe_relation(""), "RELATED_TO");
    }

    #[test]
    fn merge_adds_cited_summaries_facts_concepts_and_cross_page_relations() {
        let pages = vec![
            PageResult {
                page_num: 1,
                text: "Alpha evidence".into(),
                image_path: String::new(),
            },
            PageResult {
                page_num: 2,
                text: "Beta evidence".into(),
                image_path: String::new(),
            },
        ];
        let diagnostics = pages
            .iter()
            .map(|page| crate::OcrPageResult {
                page: page.page_num,
                chars: page.text.len(),
                columns: 1,
                headings: vec![],
                figures: vec![],
                layout_source: crate::LayoutSource::PdfText,
                warnings: vec![],
                cached: false,
            })
            .collect::<Vec<_>>();
        let mut package =
            crate::knowledge::build_knowledge_package(&pages, &diagnostics, "fixture.pdf");
        let assembly = SemanticAssembly {
            document_summary: "Alpha affects beta.".into(),
            page_summaries: vec![PageSummary {
                page: 1,
                summary: "Alpha is introduced.".into(),
            }],
            concepts: vec![
                Concept {
                    id: "alpha".into(),
                    name: "Alpha".into(),
                    description: "First concept".into(),
                    pages: vec![1],
                    confidence: 0.9,
                },
                Concept {
                    id: "beta".into(),
                    name: "Beta".into(),
                    description: "Second concept".into(),
                    pages: vec![2],
                    confidence: 0.8,
                },
            ],
            facts: vec![Fact {
                statement: "Alpha affects beta".into(),
                pages: vec![1, 2],
                concept_ids: vec!["alpha".into(), "beta".into()],
                confidence: 0.85,
            }],
            relationships: vec![Relationship {
                source_id: "alpha".into(),
                relation: "affects".into(),
                target_id: "beta".into(),
                pages: vec![1, 2],
                confidence: 0.84,
            }],
        };
        merge_assemblies(&mut package, vec![assembly], &HashSet::from([1, 2]));

        assert!(package
            .entities
            .iter()
            .any(|entity| entity.label == "Concept"));
        assert!(package.entities.iter().any(|entity| entity.label == "Fact"));
        assert!(package.relations.iter().any(|relation| {
            relation.relation == "AFFECTS" && relation.source_location.ends_with("#pages=1,2")
        }));
        assert!(package.knowledge_base_markdown.contains("Page 1 summary"));
    }
}
