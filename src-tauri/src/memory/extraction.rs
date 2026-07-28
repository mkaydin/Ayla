// ── LLM entity/fact/relation extraction ────────────────────
// Uses gemma4:e4b via Ollama for structured extraction.

use crate::memory::config::MemoryConfig;
use serde_json::Value;

#[derive(Clone)]
pub struct Extractor {
    config: MemoryConfig,
}

impl Extractor {
    pub fn new(config: MemoryConfig) -> Self {
        Self { config }
    }

    pub async fn extract_entities(&self, text: &str) -> Result<Vec<String>, String> {
        let prompt = format!(
            "Extract all named entities from this text. Return a JSON array of strings.\
             Only include: person names, project names, tools, technologies, programming\
             languages, file names, and significant concepts.\n\nText:\n{}\n\n\
             Output ONLY a JSON array. Example: [\"Rust\", \"Tauri\", \"Sherpa\"]",
            text
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
            text
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
            text
        );
        let response = self.ask_llm(&prompt).await?;
        parse_relation_array(&response)
    }

    async fn ask_llm(&self, prompt: &str) -> Result<String, String> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": self.config.llm_model,
            "messages": [{ "role": "system", "content": "You are a precise JSON extractor. Output ONLY valid JSON, nothing else." },
                         { "role": "user", "content": prompt }],
            "stream": false,
        });

        let resp = client
            .post(&format!("{}/api/chat", self.config.ollama_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("LLM: {}", e))?;

        let text = resp.text().await.map_err(|e| format!("{}", e))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| format!("JSON: {}", e))?;
        let content = json["message"]["content"].as_str().unwrap_or("").to_string();
        log::info!("LLM: {} chars response, first_100={:?}", content.len(), &content[..content.len().min(100)]);
        Ok(content)
    }
}

fn parse_json_array(response: &str) -> Result<Vec<String>, String> {
    // Try to find a JSON array in the response
    let cleaned = response.trim();
    if let Ok(v) = serde_json::from_str::<Vec<String>>(cleaned) {
        return Ok(v);
    }
    // Try to extract array between first [ and last ]
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
    } else {
        return Ok(vec![]);
    };

    let arr: Vec<Vec<Value>> = serde_json::from_str(json_str).unwrap_or_default();
    let facts = arr.into_iter().filter_map(|item| {
        if item.len() >= 4 {
            Some((
                item[0].as_str().unwrap_or("").to_string(),
                item[1].as_str().unwrap_or("").to_string(),
                item[2].as_str().unwrap_or("").to_string(),
                item[3].as_f64().unwrap_or(0.5) as f32,
            ))
        } else {
            None
        }
    }).collect();
    Ok(facts)
}

fn parse_relation_array(response: &str) -> Result<Vec<(String, String, String)>, String> {
    let cleaned = response.trim();
    let json_str = if let (Some(s), Some(e)) = (cleaned.find('['), cleaned.rfind(']')) {
        &cleaned[s..=e]
    } else {
        return Ok(vec![]);
    };

    let arr: Vec<Vec<String>> = serde_json::from_str(json_str).unwrap_or_default();
    let relations = arr.into_iter().filter_map(|item| {
        if item.len() >= 3 {
            Some((item[0].clone(), item[1].clone(), item[2].clone()))
        } else {
            None
        }
    }).collect();
    Ok(relations)
}
