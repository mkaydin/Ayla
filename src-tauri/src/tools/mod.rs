// ── Tool integration ───────────────────────────────────────

pub mod types;
pub mod ocr;
pub mod meta_search;
pub mod quick_extract;

use types::IngestionResult;

#[async_trait::async_trait]
pub trait IngestTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    async fn is_available(&self) -> bool;
    async fn run(&self, query: &str, file_path: Option<&str>) -> Result<IngestionResult, String>;
}

pub struct ToolRegistry {
    tools: Vec<Box<dyn IngestTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self { tools: vec![] };
        registry.register(Box::new(ocr::OcrTool::new()));
        registry.register(Box::new(meta_search::MetaSearchTool::new()));
        registry.register(Box::new(quick_extract::QuickExtractTool::new()));
        registry
    }

    pub fn register(&mut self, tool: Box<dyn IngestTool>) {
        self.tools.push(tool);
    }

    pub async fn execute(
        &self,
        tool_name: &str,
        query: &str,
        file_path: Option<&str>,
    ) -> Result<IngestionResult, String> {
        let tool = self.tools
            .iter()
            .find(|t| t.name() == tool_name)
            .ok_or_else(|| format!("Unknown tool: {}", tool_name))?;

        if !tool.is_available().await {
            return Err(format!("Tool '{}' is not available", tool_name));
        }

        tool.run(query, file_path).await
    }

    pub fn dispatch_prompt(user_message: &str) -> String {
        format!(
            r#"Analyze this user message and decide which knowledge tool to use.

Available tools:
- "ocr": Extract text from a PDF file. Use when the user mentions a PDF, document, or file path.
- "search": Search scientific literature. Use when the user asks to research, find papers, or survey a topic.
- "none": No tool needed. Use for normal conversation.

Return ONLY a JSON object with "tool", "query", and optional "file_path" fields.

User message: "{}"

JSON:"#,
            user_message
        )
    }
}
