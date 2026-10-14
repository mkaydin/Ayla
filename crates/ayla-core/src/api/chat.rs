//! Backend chat operations.

use super::*;

impl BackendApi<'_> {
    pub async fn enrich_prompt(
        &self,
        user_query: &str,
        base_prompt: &str,
    ) -> Result<String, BackendError> {
        self.manager
            .enrich_context(user_query, base_prompt)
            .await
            .map_err(BackendError::from)
    }

    pub async fn process_turn(
        &self,
        user_message: &str,
        assistant_message: &str,
    ) -> Result<(), BackendError> {
        self.manager
            .process_turn(user_message, assistant_message)
            .await
            .map_err(BackendError::from)
    }

    pub async fn process_message(
        &self,
        query: &str,
        base_prompt: &str,
    ) -> Result<ProcessMessageResult, BackendError> {
        let (system_prompt, tool_status) = self
            .manager
            .process_with_tools(query, base_prompt)
            .await
            .map_err(BackendError::from)?;
        Ok(ProcessMessageResult {
            system_prompt,
            tool_status,
        })
    }

    pub async fn stream_chat(
        &self,
        messages: Vec<Value>,
        stream: &dyn StreamSink,
    ) -> Result<(), BackendError> {
        let payload_size = serde_json::to_vec(&messages)
            .map_err(|error| {
                BackendError::new("invalid_payload", format!("Chat payload: {error}"))
            })?
            .len();
        if payload_size > MAX_MESSAGES_PAYLOAD {
            return Err(BackendError::new(
                "payload_too_large",
                format!(
                    "Chat payload too large ({} bytes, max {})",
                    payload_size, MAX_MESSAGES_PAYLOAD
                ),
            ));
        }
        let num_ctx = self
            .manager
            .config
            .ollama_context_for(payload_size, CHAT_MAX_OUTPUT_TOKENS);
        log::debug!(
            "Ollama chat: payload={} bytes, num_ctx={num_ctx}",
            payload_size
        );

        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| BackendError::new("chat_client", format!("Chat client: {error}")))?;
        let mut response = client
            .post(format!("{}/api/chat", self.manager.config.ollama_url))
            .json(&serde_json::json!({
                "model": self.manager.config.llm_model,
                "messages": messages,
                "stream": true,
                "options": {
                    "temperature": 0.7,
                    "num_ctx": num_ctx,
                    "num_predict": CHAT_MAX_OUTPUT_TOKENS,
                },
            }))
            .send()
            .await
            .map_err(|error| BackendError::new("ollama_request", format!("Ollama: {error}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(BackendError::new(
                "ollama_http",
                format!(
                    "Ollama HTTP {}: {}",
                    status,
                    crate::memory::safe_truncate(&body, 300)
                ),
            ));
        }

        let mut pending = Vec::<u8>::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            BackendError::new("ollama_stream", format!("Ollama stream: {error}"))
        })? {
            pending.extend_from_slice(&chunk);
            while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=newline).collect();
                let line = line.strip_suffix(b"\n").unwrap_or(&line);
                if line.is_empty() {
                    continue;
                }
                emit_chat_delta(line, stream)?;
            }
        }
        if !pending.is_empty() {
            // Ollama normally terminates records with a newline. Preserve the
            // previous tolerant behavior for a final unterminated record.
            if let Ok(value) = serde_json::from_slice::<Value>(&pending) {
                emit_chat_delta_value(&value, stream)?;
            }
        }
        stream
            .send(serde_json::json!({ "type": "done" }))
            .map_err(chat_stream_error)?;
        Ok(())
    }
}
