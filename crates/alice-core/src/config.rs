use std::collections::HashMap;
#[cfg(debug_assertions)]
use std::path::Path;
use std::path::PathBuf;

const OLLAMA_MIN_AUTO_CTX: usize = 8_192;
const OLLAMA_CONTEXT_BUCKETS: [usize; 5] = [8_192, 16_384, 32_768, 65_536, 131_072];
const APPROX_BYTES_PER_TOKEN: usize = 3;

#[derive(Clone)]
pub struct MemoryConfig {
    pub surrealdb_url: String,
    pub surrealdb_ns: String,
    pub surrealdb_db: String,
    pub surrealdb_user: String,
    pub surrealdb_pass: String,
    pub qdrant_url: String,
    pub ollama_url: String,
    pub searxng_url: String,
    pub embed_model: String,
    pub llm_model: String,
    pub ollama_num_ctx: usize,
    pub embed_dim: usize,
    pub top_k_retrieval: usize,
}

fn read_dotenv() -> HashMap<String, String> {
    let mut candidates = Vec::<PathBuf>::new();
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join(".env"));
        if let Some(parent) = current.parent() {
            candidates.push(parent.join(".env"));
        }
    }
    #[cfg(debug_assertions)]
    if let Some(project_root) = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
    {
        candidates.push(project_root.join(".env"));
    }

    let Some(path) = candidates.into_iter().find(|path| path.is_file()) else {
        return HashMap::new();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        log::warn!(
            "Could not read local environment file at {}",
            path.display()
        );
        return HashMap::new();
    };

    contents
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let (key, raw_value) = line.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let raw_value = raw_value.trim();
            let value = if raw_value.len() >= 2
                && ((raw_value.starts_with('"') && raw_value.ends_with('"'))
                    || (raw_value.starts_with('\'') && raw_value.ends_with('\'')))
            {
                &raw_value[1..raw_value.len() - 1]
            } else {
                raw_value
            };
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn env_or(default: &str, keys: &[&str], dotenv: &HashMap<String, String>) -> String {
    keys.iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .or_else(|| {
            keys.iter()
                .find_map(|key| dotenv.get(*key).filter(|value| !value.is_empty()).cloned())
        })
        .unwrap_or_else(|| default.into())
}

fn env_usize(default: usize, keys: &[&str], dotenv: &HashMap<String, String>) -> usize {
    env_or(&default.to_string(), keys, dotenv)
        .parse()
        .unwrap_or(default)
}

impl Default for MemoryConfig {
    fn default() -> Self {
        let dotenv = read_dotenv();
        Self {
            surrealdb_url: env_or("http://127.0.0.1:8000", &["SURREALDB_URL"], &dotenv),
            surrealdb_ns: env_or("alice", &["SURREALDB_NS"], &dotenv),
            surrealdb_db: env_or("memory", &["SURREALDB_DB"], &dotenv),
            surrealdb_user: env_or("root", &["SURREALDB_USER", "SURREAL_USER"], &dotenv),
            surrealdb_pass: env_or("root", &["SURREALDB_PASS", "SURREAL_PASS"], &dotenv),
            qdrant_url: env_or("http://127.0.0.1:6334", &["QDRANT_URL"], &dotenv),
            ollama_url: env_or("http://127.0.0.1:11434", &["OLLAMA_URL"], &dotenv),
            searxng_url: env_or("http://127.0.0.1:8888", &["SEARXNG_URL"], &dotenv),
            embed_model: env_or("qwen3-embedding:0.6b", &["ALICE_EMBED_MODEL"], &dotenv),
            llm_model: env_or("gemma4:e4b", &["ALICE_LLM_MODEL"], &dotenv),
            // Treat this as the hardware-safe ceiling, not a target for every call.
            ollama_num_ctx: env_usize(32_768, &["ALICE_OLLAMA_NUM_CTX"], &dotenv),
            embed_dim: 1024,
            top_k_retrieval: 5,
        }
    }
}

impl MemoryConfig {
    pub fn ollama_context_for(&self, input_bytes: usize, output_tokens: usize) -> usize {
        let input_tokens = input_bytes.div_ceil(APPROX_BYTES_PER_TOKEN);
        let safety_tokens = (input_tokens / 4).max(512);
        let required = input_tokens
            .saturating_add(output_tokens)
            .saturating_add(safety_tokens);
        let selected = OLLAMA_CONTEXT_BUCKETS
            .into_iter()
            .find(|bucket| *bucket >= required)
            .unwrap_or(self.ollama_num_ctx)
            .min(self.ollama_num_ctx);
        selected.max(OLLAMA_MIN_AUTO_CTX.min(self.ollama_num_ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_context_buckets_and_respects_gpu_cap() {
        let config = MemoryConfig {
            ollama_num_ctx: 32_768,
            ..MemoryConfig::default()
        };

        assert_eq!(config.ollama_context_for(1_000, 128), 8_192);
        assert_eq!(config.ollama_context_for(10_000, 4_096), 16_384);
        assert_eq!(config.ollama_context_for(40_000, 8_192), 32_768);

        let capped = MemoryConfig {
            ollama_num_ctx: 16_384,
            ..MemoryConfig::default()
        };
        assert_eq!(capped.ollama_context_for(40_000, 8_192), 16_384);
    }
}
