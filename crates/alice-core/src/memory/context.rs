// ── Context builder ────────────────────────────────────────
// Converts RetrievedContext into injectable prompt prefix.

use crate::memory::retrieval::RetrievedContext;
use crate::memory::safe_truncate;

pub struct ContextBuilder;

impl ContextBuilder {
    pub fn build_memory_context(context: &RetrievedContext) -> String {
        let mut prompt = String::new();

        if !context.related_facts.is_empty()
            || !context.related_episodes.is_empty()
            || !context.related_entities.is_empty()
        {
            prompt.push_str("## Current Context\n\n");
            prompt.push_str("The following information is relevant to master's query. Use it where applicable. Do not repeat it verbatim unless asked.\n\n");
        }

        if !context.related_facts.is_empty() {
            prompt.push_str("### Master's Facts\n");
            for fact in &context.related_facts {
                prompt.push_str(&format!("- {}\n", fact));
            }
            prompt.push('\n');
        }

        if !context.related_episodes.is_empty() {
            prompt.push_str("### Related Past Events\n");
            for ep in &context.related_episodes {
                prompt.push_str(&format!("- {}\n", ep));
            }
            prompt.push('\n');
        }

        if !context.related_entities.is_empty() {
            prompt.push_str("### Related Entities\n");
            for e in &context.related_entities {
                prompt.push_str(&format!("- {}\n", e));
            }
            prompt.push('\n');
        }

        if !context.related_documents.is_empty() {
            prompt.push_str("### Relevant Document Excerpts\n");
            for (i, doc) in context.related_documents.iter().enumerate() {
                prompt.push_str(&format!(
                    "[Doc chunk {}]: {}\n",
                    i + 1,
                    safe_truncate(doc, 300)
                ));
            }
            prompt.push('\n');
        }

        if !prompt.is_empty() {
            prompt.push_str("---\n\n");
        }

        prompt
    }
}
