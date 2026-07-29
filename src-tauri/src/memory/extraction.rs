// ── LLM knowledge extraction ───────────────────────────────
// Uses gemma4:e4b via Ollama for structured extraction.
// Single consolidated call: entities + relations + facts.

use crate::memory::config::MemoryConfig;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct Extractor {
    config: MemoryConfig,
    client: reqwest::Client,
}

pub struct KnowledgeExtract {
    pub entities: Vec<String>,
    pub facts: Vec<(String, String, String, f32)>,
    pub relations: Vec<(String, String, String)>,
}

impl Extractor {
    pub fn new(config: MemoryConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    pub async fn extract_all(&self, text: &str) -> Result<KnowledgeExtract, String> {
        let prompt = format!(
            "Analyze this document and extract knowledge as JSON.\n\n\
             Extract:\n\
             - \"entities\": names of people, projects, tools, technologies, files, concepts\n\
             - \"facts\": facts about the document's author/master. Each is a 4-element tuple:\
               [subject, predicate, object, confidence]. Subject is \"master\" or an entity name.\
               Confidence 0.8-1.0 for explicit, 0.5-0.7 for implied.\n\
             - \"relations\": relationships between entities. Each is a 3-element tuple:\
               [from_entity, relation_type, to_entity].\
               Relation types: USES, WORKS_ON, PREFERS, CREATED, KNOWS, DEPENDS_ON,\
               INTEGRATES_WITH, REFERENCES, OWNS, CONTAINS, CONTRIBUTES_TO, IMPLEMENTS, DESCRIBES.\n\n\
             CRITICAL RULES:\n\
             1. Every entity in \"entities\" MUST appear in at least one \"relations\" tuple.\
                If an entity has no meaningful relationship, do NOT include it.\n\
             2. Entities connected to the document itself should use \"this_document\" as one side.\n\
             3. Infer implicit relationships: if A uses tool T and B also works on T, connect A and B.\n\
             4. Be conservative: prefer fewer well-connected entities over many isolated ones.\n\n\
             Document text:\n{}\n\n\
             Output ONLY a JSON object. Format:\n\
             {{\"entities\":[\"Name\",\"Tool\"],\
               \"facts\":[[\"subject\",\"predicate\",\"object\",0.9]],\
               \"relations\":[[\"Name\",\"USES\",\"Tool\"]]}}",
            &text[..text.len().min(6000)]
        );
        let response = self.ask_llm(&prompt).await?;
        parse_knowledge(&response)
    }

    pub async fn extract_entities(&self, text: &str) -> Result<Vec<String>, String> {
        let prompt = format!(
            "Extract all named entities from this text. Return a JSON array of strings.\
             Only include: person names, project names, tools, technologies, programming\
             languages, file names, and significant concepts.\
             Do NOT include entities that have no relationship to anything else in the text.\n\n\
             Text:\n{}\n\n\
             Output ONLY a JSON array. Example: [\"Rust\", \"Tauri\", \"Sherpa\"]",
            &text[..text.len().min(4000)]
        );
        let response = self.ask_llm(&prompt).await?;
        parse_json_array(&response)
    }

    pub async fn extract_facts(&self, text: &str) -> Result<Vec<(String, String, String, f32)>, String> {
        let prompt = format!(
            "Extract facts from this text. Each fact is (subject, predicate, object, confidence).\
             Subject is usually \"master\" or an entity name. Predicate describes the relationship\
             (prefers, uses, works_on, knows, etc.). Confidence: 1.0 for explicit statements,\
             0.5-0.9 for implied, below 0.5 omit.\n\nText:\n{}\n\n\
             Output ONLY a JSON array of [subject, predicate, object, confidence].\n\
             Example: [[\"master\", \"prefers\", \"Rust\", 0.9]]",
            &text[..text.len().min(4000)]
        );
        let response = self.ask_llm(&prompt).await?;
        parse_fact_array(&response)
    }

    pub async fn extract_relations(&self, text: &str) -> Result<Vec<(String, String, String)>, String> {
        let prompt = format!(
            "Extract relationships between entities from this text.\
             Relation types: USES, PREFERS, WORKS_ON, KNOWS, OWNS, CREATED, STUDIED,\
             REFERENCES, DEPENDS_ON, INTEGRATES_WITH, REPLACES, CONTAINS.\n\n\
             Text:\n{}\n\n\
             Output ONLY a JSON array of [from_entity, relation_type, to_entity].\n\
             Example: [[\"master\", \"USES\", \"Sherpa-ONNX\"]]",
            &text[..text.len().min(4000)]
        );
        let response = self.ask_llm(&prompt).await?;
        parse_relation_array(&response)
    }

    async fn ask_llm(&self, prompt: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "model": self.config.llm_model,
            "messages": [{ "role": "system", "content": "You are a precise JSON knowledge extractor. Output ONLY valid JSON objects, nothing else." },
                         { "role": "user", "content": prompt }],
            "stream": false,
        });

        let resp = self.client
            .post(&format!("{}/api/chat", self.config.ollama_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM: {}", e))?;

        let text = resp.text().await.map_err(|e| format!("{}", e))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| format!("JSON: {}", e))?;
        let content = json["message"]["content"].as_str().unwrap_or("").to_string();
        log::info!("LLM extract: {} chars", content.len());
        Ok(content)
    }
}

fn parse_knowledge(response: &str) -> Result<KnowledgeExtract, String> {
    let cleaned = response.trim();
    let json_str = if let (Some(s), Some(e)) = (cleaned.find('{'), cleaned.rfind('}')) {
        &cleaned[s..=e]
    } else {
        return Ok(KnowledgeExtract { entities: vec![], facts: vec![], relations: vec![] });
    };

    let obj: Value = serde_json::from_str(json_str).unwrap_or(Value::Null);

    let entities: Vec<String> = obj.get("entities")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let facts: Vec<(String, String, String, f32)> = obj.get("facts")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| {
            let arr = v.as_array()?;
            if arr.len() >= 4 {
                Some((
                    arr[0].as_str().unwrap_or("").to_string(),
                    arr[1].as_str().unwrap_or("").to_string(),
                    arr[2].as_str().unwrap_or("").to_string(),
                    arr[3].as_f64().unwrap_or(0.5) as f32,
                ))
            } else { None }
        }).collect())
        .unwrap_or_default();

    let relations: Vec<(String, String, String)> = obj.get("relations")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| {
            let arr = v.as_array()?;
            if arr.len() >= 3 {
                Some((
                    arr[0].as_str().unwrap_or("").to_string(),
                    arr[1].as_str().unwrap_or("").to_string(),
                    arr[2].as_str().unwrap_or("").to_string(),
                ))
            } else { None }
        }).collect())
        .unwrap_or_default();

    log::info!("Extracted: {} entities, {} facts, {} relations", entities.len(), facts.len(), relations.len());
    Ok(KnowledgeExtract { entities, facts, relations })
}

fn parse_json_array(response: &str) -> Result<Vec<String>, String> {
    let cleaned = response.trim();
    if let Ok(v) = serde_json::from_str::<Vec<String>>(cleaned) {
        return Ok(v);
    }
    if let (Some(start), Some(end)) = (cleaned.find('['), cleaned.rfind(']')) {
        if let Ok(v) = serde_json::from_str::<Vec<String>>(&cleaned[start..=end]) {
            return Ok(v);
        }
    }
    Ok(vec![])
}

fn parse_fact_array(response: &str) -> Result<Vec<(String, String, String, f32)>, String> {
    let cleaned = response.trim();
    let json_str = if let (Some(s), Some(e)) = (cleaned.find('['), cleaned.rfind(']')) {
        &cleaned[s..=e]
    } else { return Ok(vec![]); };
    let arr: Vec<Vec<Value>> = serde_json::from_str(json_str).unwrap_or_default();
    let facts = arr.into_iter().filter_map(|item| {
        if item.len() >= 4 {
            Some((item[0].as_str().unwrap_or("").to_string(), item[1].as_str().unwrap_or("").to_string(), item[2].as_str().unwrap_or("").to_string(), item[3].as_f64().unwrap_or(0.5) as f32))
        } else { None }
    }).collect();
    Ok(facts)
}

fn parse_relation_array(response: &str) -> Result<Vec<(String, String, String)>, String> {
    let cleaned = response.trim();
    let json_str = if let (Some(s), Some(e)) = (cleaned.find('['), cleaned.rfind(']')) {
        &cleaned[s..=e]
    } else { return Ok(vec![]); };
    let arr: Vec<Vec<String>> = serde_json::from_str(json_str).unwrap_or_default();
    let relations = arr.into_iter().filter_map(|item| {
        if item.len() >= 3 { Some((item[0].clone(), item[1].clone(), item[2].clone())) } else { None }
    }).collect();
    Ok(relations)
}
