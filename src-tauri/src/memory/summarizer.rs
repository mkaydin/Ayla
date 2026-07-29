// ── LLM conversation summarization ────────────────────────
// Uses gemma4:e4b via Ollama.

use crate::memory::config::MemoryConfig;
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct Summarizer {
    config: MemoryConfig,
    client: reqwest::Client,
}

impl Summarizer {
    pub fn new(config: MemoryConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { config, client }
    }

    pub async fn summarize_turn(&self, user_msg: &str, assistant_msg: &str) -> Result<String, String> {
        let prompt = format!(
            "Summarize this conversation exchange in 1-2 sentences. Be factual and concise.\
             Only output the summary, nothing else.\n\n\
             Master: {}\nAlice: {}\n\nSummary:",
            user_msg, assistant_msg
        );
        self.ask_llm(&prompt).await
    }

    pub async fn summarize_session(&self, messages: &[String]) -> Result<String, String> {
        let joined = messages.join("\n");
        let prompt = format!(
            "Summarize this full conversation session into a paragraph for memory.\
             Include: what master wanted, what was discussed, what was decided,\
             and any actions taken. Keep it factual.\n\n\
             Conversation:\n{}\n\nSession summary:",
            joined
        );
        self.ask_llm(&prompt).await
    }

    pub async fn score_importance(&self, user_msg: &str, assistant_msg: &str) -> Result<f32, String> {
        let prompt = format!(
            "Rate the importance of this conversation from 0.0 (trivial) to 1.0 (critical).\
             Consider: explicit \"remember\" requests (+0.4), personal preferences (+0.3),\
             emotional significance (+0.2), technical facts (+0.2).\
             Small talk is 0.0-0.1.\n\n\
             Return ONLY a number like 0.7\n\n\
             Master: {}\nAlice: {}\n\nImportance:",
            user_msg, assistant_msg
        );
        let response = self.ask_llm(&prompt).await?;
        Ok(response.trim().parse::<f32>().unwrap_or(0.3))
    }

    async fn ask_llm(&self, prompt: &str) -> Result<String, String> {
        let body = serde_json::json!({
            "model": self.config.llm_model,
            "messages": [{ "role": "system", "content": "You are a precise extraction model. Output ONLY the requested value, nothing else." },
                         { "role": "user", "content": prompt }],
            "stream": false,
        });

        let resp = self.client.post(&format!("{}/api/chat", self.config.ollama_url)).json(&body).send().await
            .map_err(|e| format!("LLM: {}", e))?;
        let text = resp.text().await.map_err(|e| format!("{}", e))?;
        let json: Value = serde_json::from_str(&text).map_err(|e| format!("JSON: {}", e))?;
        Ok(json["message"]["content"].as_str().unwrap_or("").trim().to_string())
    }
}
