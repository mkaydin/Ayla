# Information Gathering Tools — Integration Plan

## Integrating `ocr_helper_system` and `meta_search_Js_render` into Alice's Memory System

---

## 1. What We Have

### `ocr_helper_system` — PDF OCR Tool
- **Location:** `/home/mka/Code/ocr_helper_system`
- **Binary:** `ocr-helper` (release binary at `target/release/ocr-helper`)
- **Function:** PDF → render pages as images → Ollama deepseek-ocr → cleaned Markdown
- **Tech:** Rust, pdfium, image, reqwest, clap
- **Depends on:** Ollama with `deepseek-ocr` model (~6.7 GB, needs pulling), PDFium (bundled)
- **Output:** `{output}.md` file + `{output}_images/` directory

### `meta_search_Js_render` — Scientific Literature Search
- **Location:** `/home/mka/Code/meta_search_Js_render`
- **Binary:** `meta_search` (release binary at `target/release/meta_search`)
- **Function:** Query enrichment → SearXNG search (8 engines) → PDF download + JS rendering → text extraction → AI paper analysis → cross-paper synthesis → JSON + Markdown report
- **Tech:** Rust, lopdf, reqwest, tokio-tungstenite, clap
- **Depends on:** Docker (SearXNG + Lightpanda containers), Ollama with `gemma4:e4b`
- **Output:** `reports/<uuid>.json` + `reports/<uuid>.md` + `downloads/<uuid>/` (PDFs)

### `Alice_AI_Demoness` — AI Assistant
- **Location:** `/home/mka/Code/Alice_AI_Demoness`
- **Function:** Chat interface with Live2D avatar, TTS, LLM via Ollama
- **Tech:** Tauri 2 (Rust backend + JS frontend), sherpa-onnx, pixi.js
- **Planned:** Memory system with SurrealDB + Qdrant + Petgraph (see `memory_implementation_plan.md`)

---

## 2. Integration Architecture

```
┌───────────────────────────────────────────────────────────┐
│                    Tauri App (Rust + JS)                   │
│                                                           │
│  ┌─────────────────┐    ┌──────────────────────────────┐ │
│  │   Chat Pipeline  │    │      Tool Dispatch Layer     │ │
│  │  (main.js + Rust)│    │     (new Rust module)        │ │
│  └────────┬────────┘    └──────────┬───────────────────┘ │
│           │                        │                      │
│           │  user asks:            │  tool call:          │
│           │  "analyze this PDF"    │  ocr_helper          │
│           │  "search papers on X"  │  meta_search         │
│           │                        │                      │
│  ┌────────▼────────────────────────▼───────────────────┐  │
│  │              Memory Pipeline (existing plan)          │  │
│  │  Summarizer → Extractor → Embedder → Router → DBs    │  │
│  └──────────────────────┬──────────────────────────────┘  │
│                         │                                 │
└─────────────────────────┼─────────────────────────────────┘
                          │
          ┌───────────────┼───────────────┐
          │               │               │
    ┌─────▼─────┐  ┌──────▼──────┐  ┌────▼──────┐
    │ SurrealDB │  │   Qdrant    │  │  Ollama   │
    │  :8000    │  │ :6333/6334  │  │  :11434   │
    └───────────┘  └─────────────┘  └───────────┘
```

**Key principle:** The tools run as **subprocesses** (keeping them independent), but their output is parsed and fed directly into the **memory pipeline** so extracted knowledge persists across conversations.

---

## 3. New Docker Services (Phase 1)

Add to the existing `docker-compose.yml` in the project root:

```yaml
version: "3.9"

services:
  # --- Memory System (existing from memory_implementation_plan.md) ---
  surrealdb:
    image: surrealdb/surrealdb:latest
    container_name: alice-surrealdb
    ports: ["8000:8000"]
    volumes: ["./data/surrealdb:/data"]
    command: start --log trace --user root --pass root file:/data/memory.db
    restart: unless-stopped

  qdrant:
    image: qdrant/qdrant:latest
    container_name: alice-qdrant
    ports: ["6333:6333", "6334:6334"]
    volumes: ["./data/qdrant:/qdrant/storage"]
    restart: unless-stopped

  # --- Meta-Search Services (NEW) ---
  searxng:
    image: searxng/searxng:latest
    container_name: alice-searxng
    ports: ["8888:8080"]
    volumes:
      - ./searxng/settings.yml:/etc/searxng/settings.yml:ro
      - ./searxng/limiter.toml:/etc/searxng/limiter.toml:ro
      - ./data/searxng:/etc/searxng
    restart: unless-stopped
    cap_drop: [ALL]
    cap_add: [CHOWN, SETGID, SETUID]

  lightpanda:
    image: lightpanda/browser:latest
    container_name: alice-lightpanda
    ports: ["9222:9222"]
    restart: unless-stopped
```

Copy SearXNG config files from `meta_search_Js_render`:

```bash
mkdir -p searxng
cp /home/mka/Code/meta_search_Js_render/searxng/settings.yml searxng/
cp /home/mka/Code/meta_search_Js_render/searxng/limiter.toml searxng/
```

---

## 4. New Rust Modules (Phase 2)

Add under `src-tauri/src/`:

```
src-tauri/src/
│
├── tools/                          # NEW — external tool integration
│   ├── mod.rs                      # ToolRegistry + Tool trait + dispatch
│   ├── ocr.rs                      # ocr-helper subprocess wrapper
│   ├── meta_search.rs             # meta_search subprocess wrapper
│   └── types.rs                    # Shared tool result types
│
├── memory/                         # (existing plan — unchanged)
│   ├── ingestion.rs                # NEW — feed tool results into memory pipeline
│   └── ...
```

---

## 5. Tool Abstraction Layer

### 5.1 — `tools/types.rs`

Unified result type shared across all tools:

```rust
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Standardized output from any knowledge ingestion tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestionResult {
    /// Human-readable summary of what was ingested
    pub summary: String,

    /// Extracted entities (people, projects, tools, concepts)
    pub entities: Vec<String>,

    /// Extracted facts (subject, predicate, object, confidence)
    pub facts: Vec<Fact>,

    /// Relationships between entities
    pub relations: Vec<Relation>,

    /// Full extracted text (for embedding + conversation storage)
    pub full_text: String,

    /// Source metadata (file path, search query, URL, etc.)
    pub source: SourceInfo,

    /// When ingestion happened
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    pub from: String,
    pub relation_type: String,
    pub to: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub tool: String,            // "ocr_helper" | "meta_search"
    pub input: String,           // file path | search query
    pub output_path: Option<String>, // path to saved file if applicable
    pub metadata: serde_json::Value, // tool-specific extra info
}

/// What the LLM extracts from the user message to decide which tool to call.
#[derive(Debug, Deserialize)]
pub struct ToolDispatch {
    pub tool: String,            // "ocr" | "search" | "none"
    pub query: String,           // the parsed query/path for the tool
    pub file_path: Option<String>, 
}
```

### 5.2 — `tools/mod.rs`

Tool trait and registry:

```rust
pub mod ocr;
pub mod meta_search;
pub mod types;

use types::{IngestionResult, ToolDispatch};

/// Every ingest tool implements this trait.
#[async_trait::async_trait]
pub trait IngestTool: Send + Sync {
    /// Unique name for dispatch (e.g., "ocr", "search")
    fn name(&self) -> &str;

    /// Description for the LLM to understand when to use this tool.
    fn description(&self) -> &str;

    /// Is this tool's external dependencies available?
    async fn is_available(&self) -> bool;

    /// Run the tool and return structured ingestion results.
    async fn run(&self, query: &str, file_path: Option<&str>) -> Result<IngestionResult, String>;
}

/// Registry of all available ingest tools.
pub struct ToolRegistry {
    tools: Vec<Box<dyn IngestTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self { tools: vec![] };
        registry.register(Box::new(ocr::OcrTool::new()));
        registry.register(Box::new(meta_search::MetaSearchTool::new()));
        registry
    }

    pub fn register(&mut self, tool: Box<dyn IngestTool>) {
        self.tools.push(tool);
    }

    /// Ask the LLM to decide which tool to use based on user intent.
    pub async fn dispatch(&self, user_message: &str) -> Result<ToolDispatch, String>;

    /// Execute a dispatched tool and return results.
    pub async fn execute(
        &self,
        tool_name: &str,
        query: &str,
        file_path: Option<&str>,
    ) -> Result<IngestionResult, String> {
        let tool = self.tools.iter()
            .find(|t| t.name() == tool_name)
            .ok_or_else(|| format!("Unknown tool: {}", tool_name))?;

        if !tool.is_available().await {
            return Err(format!("Tool '{}' is not available", tool_name));
        }

        tool.run(query, file_path).await
    }

    /// Build a prompt for the LLM to decide tool dispatch.
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
```

### 5.3 — `tools/ocr.rs`

Subprocess wrapper for `ocr-helper`:

```rust
use super::types::*;
use super::IngestTool;
use std::path::PathBuf;
use tokio::process::Command;

pub struct OcrTool {
    bin_path: PathBuf,  // path to ocr-helper binary
}

impl OcrTool {
    pub fn new() -> Self {
        Self {
            // Look for binary in standard locations or build it if missing
            bin_path: Self::find_binary(),
        }
    }

    fn find_binary() -> PathBuf {
        // 1. Check if built in external project
        let external = PathBuf::from("../ocr_helper_system/target/release/ocr-helper");
        if external.exists() { return external; }

        // 2. Check PATH
        PathBuf::from("ocr-helper")
    }
}

#[async_trait::async_trait]
impl IngestTool for OcrTool {
    fn name(&self) -> &str { "ocr" }

    fn description(&self) -> &str {
        "Extracts text from PDF documents using OCR. Call when the user mentions a PDF file."
    }

    async fn is_available(&self) -> bool {
        self.bin_path.exists() || which::which("ocr-helper").is_ok()
    }

    async fn run(&self, query: &str, file_path: Option<&str>) -> Result<IngestionResult, String> {
        let input = file_path.unwrap_or(query);
        let output_stem = format!("/tmp/alice_ocr_{}", uuid::Uuid::new_v4());
        let output_md = format!("{}.md", output_stem);

        // Run ocr-helper as subprocess
        let output = Command::new(&self.bin_path)
            .args(["--input", input, "--output", &output_md, "--count", "0", "--no-images"])
            .output()
            .await
            .map_err(|e| format!("Failed to run ocr-helper: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ocr-helper failed: {}", stderr));
        }

        // Read the generated markdown
        let full_text = tokio::fs::read_to_string(&output_md)
            .await
            .map_err(|e| format!("Failed to read OCR output: {}", e))?;

        // The extracted text becomes input to the memory pipeline (entities/facts/relations
        // will be extracted by the LLM-based extractor in the memory pipeline, not here).
        Ok(IngestionResult {
            summary: format!("OCR extracted {} chars from '{}'", full_text.len(), input),
            entities: vec![],     // filled by memory pipeline extractor
            facts: vec![],        // filled by memory pipeline extractor
            relations: vec![],    // filled by memory pipeline extractor
            full_text,
            source: SourceInfo {
                tool: "ocr_helper".into(),
                input: input.into(),
                output_path: Some(output_md),
                metadata: serde_json::json!({"file": input}),
            },
            timestamp: chrono::Utc::now(),
        })
    }
}
```

### 5.4 — `tools/meta_search.rs`

Subprocess wrapper for `meta_search`:

```rust
use super::types::*;
use super::IngestTool;
use std::path::PathBuf;
use tokio::process::Command;

pub struct MetaSearchTool {
    bin_path: PathBuf,
}

impl MetaSearchTool {
    pub fn new() -> Self {
        Self { bin_path: Self::find_binary() }
    }

    fn find_binary() -> PathBuf {
        let external = PathBuf::from("../meta_search_Js_render/target/release/meta_search");
        if external.exists() { return external; }
        PathBuf::from("meta_search")
    }
}

#[async_trait::async_trait]
impl IngestTool for MetaSearchTool {
    fn name(&self) -> &str { "search" }

    fn description(&self) -> &str {
        "Searches scientific literature across multiple engines, downloads and analyzes papers. \
         Call when the user wants to research a topic, find papers, or survey a field."
    }

    async fn is_available(&self) -> bool {
        self.bin_path.exists() || which::which("meta_search").is_ok()
    }

    async fn run(&self, query: &str, _file_path: Option<&str>) -> Result<IngestionResult, String> {
        // Create a session-specific output directory
        let session_id = uuid::Uuid::new_v4().to_string();
        let reports_dir = format!("/tmp/alice_search_{}", session_id);

        // Run meta_search with JSON output
        let output = Command::new(&self.bin_path)
            .args([
                "-j",           // JSON output to stdout
                "-n", "10",     // max 10 papers
                "--model", "gemma4:e4b",
                "--searxng-url", "http://localhost:8888",
                "--lightpanda-url", "ws://localhost:9222",
                query,
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to run meta_search: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("meta_search failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        // Parse the JSON output to extract synthesis + paper summaries
        let parsed: serde_json::Value = serde_json::from_str(&stdout)
            .map_err(|e| format!("Failed to parse meta_search output: {}", e))?;

        // Build summary from the synthesis section
        let synthesis = parsed["synthesis"]["summary"]
            .as_str()
            .unwrap_or("No synthesis available")
            .to_string();

        // Combine all paper analyses into full_text for memory ingestion
        let mut full_text = String::new();
        full_text.push_str(&format!("# Research Query: {}\n\n", query));
        full_text.push_str(&format!("## Synthesis\n{}\n\n", synthesis));

        if let Some(papers) = parsed["papers"].as_array() {
            for paper in papers {
                let title = paper["title"].as_str().unwrap_or("Untitled");
                let findings = paper["key_findings"]
                    .as_array()
                    .map(|a| a.iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join("; "))
                    .unwrap_or_default();
                full_text.push_str(&format!(
                    "## {}\nFindings: {}\n\n", title, findings
                ));
            }
        }

        // Extract paper titles as entities
        let entities: Vec<String> = parsed["papers"]
            .as_array()
            .map(|a| a.iter()
                .filter_map(|p| p["title"].as_str().map(String::from))
                .collect())
            .unwrap_or_default();

        Ok(IngestionResult {
            summary: format!("Searched '{}': found {} papers. Synthesis: {}",
                query,
                entities.len(),
                &synthesis[..synthesis.len().min(200)]
            ),
            entities,
            facts: vec![],         // filled by memory pipeline
            relations: vec![],     // filled by memory pipeline
            full_text,
            source: SourceInfo {
                tool: "meta_search".into(),
                input: query.into(),
                output_path: None,
                metadata: serde_json::json!({
                    "session_id": session_id,
                    "query": query,
                }),
            },
            timestamp: chrono::Utc::now(),
        })
    }
}
```

---

## 6. Memory Pipeline Integration (Phase 3)

### 6.1 — `memory/ingestion.rs`

Bridges tool results into the memory stores:

```rust
use crate::tools::types::IngestionResult;
use crate::memory::{
    conversation::ConversationStore,
    episodic::EpisodeStore,
    semantic::SemanticStore,
    graph::KnowledgeGraph,
    embeddings::Embedder,
    extraction::Extractor,
    summarizer::Summarizer,
};

pub struct IngestionPipeline {
    conversation: ConversationStore,
    episode: EpisodeStore,
    semantic: SemanticStore,
    graph: KnowledgeGraph,
    embedder: Embedder,
    extractor: Extractor,
    summarizer: Summarizer,
}

impl IngestionPipeline {
    /// Ingest tool output into all memory layers.
    pub async fn ingest(&mut self, result: IngestionResult) -> Result<(), String> {
        // 1. Save full text as conversation record (with special session)
        let session_id = format!("ingest_{}", chrono::Utc::now().timestamp());
        self.conversation
            .save_turn(&session_id, "tool", &result.full_text)
            .await
            .map_err(|e| e.to_string())?;

        // 2. Run LLM extraction on the tool output
        let entities = self.extractor.extract_entities(&result.full_text).await?;
        let facts = self.extractor.extract_facts(&result.full_text).await?;
        let relations = self.extractor.extract_relations(&result.full_text).await?;

        // 3. Update knowledge graph
        for entity in &entities {
            self.graph.upsert_node(entity, crate::memory::graph::NodeType::Concept);
        }
        for (from, rel, to) in &relations {
            let fi = self.graph.upsert_node(from, crate::memory::graph::NodeType::Concept);
            let ti = self.graph.upsert_node(to, crate::memory::graph::NodeType::Concept);
            self.graph.upsert_edge(fi, ti, rel, 0.8);
        }

        // 4. Store facts in semantic memory
        for (subj, pred, obj, conf) in &facts {
            self.semantic.upsert_fact(subj, pred, obj, *conf).await?;
        }

        // 5. Embed and store in Qdrant for semantic retrieval
        let summary = self.summarizer.summarize_turn(
            &result.source.input,
            &result.full_text[..result.full_text.len().min(4000)]
        ).await?;
        let embedding = self.embedder.embed(&summary).await?;

        // Store as episodic memory (important event)
        self.episode.store(&result.summary, 0.7, &entities).await?;

        // 6. Sync graph to SurrealDB
        self.graph.sync().await.map_err(|e| e.to_string())?;

        Ok(())
    }
}
```

### 6.2 — Integration with `MemoryManager` (`memory/mod.rs` addendum)

```rust
// Add to MemoryManager struct:
pub struct MemoryManager {
    // ... existing fields ...
    pub tool_registry: tools::ToolRegistry,
    pub ingestion: memory::ingestion::IngestionPipeline,
}

impl MemoryManager {
    /// New method: process a user message that may contain a tool request.
    pub async fn process_with_tools(
        &self,
        user_message: &str,
        base_prompt: &str,
    ) -> Result<(String, Option<String>), String> {
        // 1. Ask LLM: does this message need a tool?
        let dispatch_prompt = ToolRegistry::dispatch_prompt(user_message);
        let dispatch_json = self.ask_llm_json(&dispatch_prompt).await?;

        let dispatch: ToolDispatch = serde_json::from_str(&dispatch_json)
            .map_err(|e| format!("Failed to parse tool dispatch: {}", e))?;

        // 2. If tool needed, run it and ingest results
        if dispatch.tool != "none" {
            let result = self.tool_registry
                .execute(&dispatch.tool, &dispatch.query, dispatch.file_path.as_deref())
                .await?;

            // Ingest in background (don't block the chat)
            // The user gets an immediate response about what's happening
            let summary = result.summary.clone();
            let mut ingestion = /* clone or Arc<Mutex<>> */ ;
            tokio::spawn(async move {
                if let Err(e) = ingestion.ingest(result).await {
                    log::error!("Ingestion failed: {}", e);
                }
            });

            return Ok((
                base_prompt.to_string(),
                Some(format!(
                    "I'll process that. {} Results will be available in my memory for future questions.",
                    summary
                ))
            ));
        }

        // 3. Normal flow: enrich context from memory
        let enriched = self.enrich_context(user_message, base_prompt).await?;
        Ok((enriched, None))
    }
}
```

---

## 7. Chat Pipeline Integration (Phase 4)

### 7.1 — Updated `chatSendMessage()` flow in `main.js`

```
User types message
        │
        ▼
  chatSendMessage()
        │
        ▼
  invoke('process_message', { query, basePrompt })  ← NEW Tauri command
        │
        ├── LLM decides: is this a tool request?
        │       │
        │       ├── YES (ocr/search)
        │       │       │
        │       │       ├── Run tool (subprocess)
        │       │       ├── Ingest results (background)
        │       │       └── Return immediate status to user
        │       │
        │       └── NO (normal chat)
        │               │
        │               ├── Enrich prompt with memory context
        │               └── Return enriched system prompt
        │
        ▼
  Continue normal streaming chat (Ollama generates response)
        │
        ▼
  After response: invoke('process_turn', { userMsg, assistantMsg })
        │ (runs memory pipeline: summarize, extract, store)
```

### 7.2 — New Tauri command in `memory/commands.rs`

```rust
/// Combined tool + memory command. Called before each chat turn.
/// Returns (enriched_system_prompt, optional_tool_status_message).
#[tauri::command]
pub async fn process_message(
    manager: State<'_, MemoryManager>,
    query: String,
    base_prompt: String,
) -> Result<serde_json::Value, String> {
    let (enriched_prompt, tool_status) = manager
        .process_with_tools(&query, &base_prompt)
        .await
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "system_prompt": enriched_prompt,
        "tool_status": tool_status,
    }))
}
```

### 7.3 — `lib.rs` changes

```rust
use memory::MemoryManager;
use memory::config::MemoryConfig;

// Register new command:
.invoke_handler(tauri::generate_handler![
    tts::generate_tts,
    memory::commands::process_message,   // NEW
    memory::commands::process_turn,       // existing
])
```

---

## 8. Dependencies to Add (Cargo.toml)

```toml
# Tool subprocess execution
async-trait = "0.1"
which = "7"

# Already planned (from memory_implementation_plan.md):
# surrealdb, qdrant-client, petgraph, tokio, reqwest, uuid, chrono
```

---

## 9. Ollama Model Requirements

| Model | Purpose | Size | Already Pulled? |
|-------|---------|------|-----------------|
| `gemma4:e4b` | Chat + extraction + summarization | 9.6 GB | Yes |
| `qwen3-embedding:0.6b` | Embedding generation | 639 MB | Yes |
| `deepseek-ocr` | PDF OCR (for ocr_helper tool) | ~6.7 GB | **No — needs `ollama pull deepseek-ocr`** |

---

## 10. Startup Sequence

```bash
# 1. Start all Docker services (4 containers)
docker compose up -d
# → surrealdb (8000), qdrant (6333/6334), searxng (8888), lightpanda (9222)

# 2. Ensure Ollama is running and models are pulled
ollama serve &
ollama pull deepseek-ocr    # first time only (~6.7 GB)

# 3. Build external tool binaries (if not already built)
(cd /home/mka/Code/ocr_helper_system && cargo build --release)
(cd /home/mka/Code/meta_search_Js_render && cargo build --release)

# 4. Start Alice
npm run tauri dev
```

---

## 11. Example Chat Flows

### Flow A: Document Ingestion

```
User:   "Can you analyze the PDF at ~/Downloads/paper.pdf?"

Alice:  "I'll process that PDF for you. OCR extracted 15,432 characters
         from paper.pdf. Results will be available in my memory."

        [Background]:
        - ocr-helper extracts text from paper.pdf
        - Memory pipeline: summarize → extract entities (authors, methods, tools) →
          extract facts (this paper uses method X) → embed → store in graph + DB

User:   "What did that paper say about neural networks?"

Alice:  [Retrieves paper knowledge from memory]
        "The paper discusses neural network architectures for... [detailed answer]"
```

### Flow B: Literature Search

```
User:   "Search for recent papers on attention mechanisms in transformers."

Alice:  "Searching scientific literature for 'attention mechanisms in
         transformers'... Analyzing 10 papers across Google Scholar, arXiv,
         PubMed, and Semantic Scholar. This may take a minute."

        [Background]:
        - meta_search runs: query enrichment → SearXNG search → PDF download →
          JS rendering → text extraction → gemma4:e4b paper analysis → synthesis
        - Memory pipeline ingests: paper titles, findings, synthesis as graph nodes
          and semantic facts

Alice:  "Synthesis complete. I found key themes: multi-head attention optimization,
         sparse attention patterns, and linear-complexity variants. I'll remember
         these for future questions. Ask me anything about these papers!"

User:   "Tell me about the linear-complexity approaches you found."

Alice:  [Retrieves from memory graph]
        "Several papers explored linear attention variants. The Linformer approach
         reduces complexity from O(n²) to O(n) by..."
```

### Flow C: Normal Chat (no tool)

```
User:   "What's your name?"

Alice:  [LLM dispatch: tool="none"]
        [Enriches prompt with memory context (previous conversations, user facts)]
        "I'm Alice, your demon girl assistant! [love] How can I help you today?"
```

---

## 12. File Manifest

```
NEW FILES:
├── docker-compose.yml                  # Updated: SurrealDB + Qdrant + SearXNG + Lightpanda
├── searxng/
│   ├── settings.yml                    # Copied from meta_search_Js_render
│   └── limiter.toml                    # Copied from meta_search_Js_render
├── src-tauri/src/tools/
│   ├── mod.rs                          # ToolRegistry + IngestTool trait + dispatch
│   ├── types.rs                        # IngestionResult, Fact, Relation, SourceInfo, ToolDispatch
│   ├── ocr.rs                          # OcrTool (ocr-helper subprocess wrapper)
│   └── meta_search.rs                 # MetaSearchTool (meta_search subprocess wrapper)
├── src-tauri/src/memory/
│   └── ingestion.rs                    # IngestionPipeline (tool results → memory pipeline)

MODIFIED FILES (from memory_implementation_plan.md):
├── src-tauri/Cargo.toml                # Add async-trait, which
├── src-tauri/src/lib.rs                # Init ToolRegistry, register process_message command
├── src-tauri/src/memory/mod.rs         # Add process_with_tools(), ingestion field
├── src-tauri/src/memory/commands.rs    # Add process_message command
├── src/main.js                         # Update chatSendMessage to call process_message

.gitignore additions:
data/stt_models/
data/searxng/
```

---

## 13. Implementation Order (additive to memory plan)

```
1. docker-compose.yml update      (10 min)   ← Add SearXNG + Lightpanda + config files
2. Cargo.toml additions           (5 min)    ← async-trait, which
3. tools/types.rs                 (15 min)   ← Shared types
4. tools/mod.rs                   (25 min)   ← ToolRegistry + dispatch + IngestTool trait
5. tools/ocr.rs                   (25 min)   ← Subprocess wrapper
6. tools/meta_search.rs           (30 min)   ← Subprocess wrapper (complex JSON parsing)
7. memory/ingestion.rs            (30 min)   ← Bridge: tool results → memory pipeline
8. memory/commands.rs update      (15 min)   ← process_message command
9. memory/mod.rs update           (20 min)   ← process_with_tools method
10. lib.rs update                  (10 min)   ← Register command, init ToolRegistry
11. main.js integration            (20 min)   ← process_message + tool status display
12. pull deepseek-ocr model        (5 min)    ← ollama pull deepseek-ocr
13. end-to-end test                (20 min)   ← Test all 3 flows
───────────────────────────────────────────
Total additional: ~4 hours
```

---

## 14. Risk Mitigation

| Risk | Mitigation |
|------|-----------|
| `deepseek-ocr` too large for system | OCR optional; check available disk before pulling |
| meta_search takes >5 min | Run asynchronously; user gets immediate status, results arrive later |
| SearXNG/Lightpanda crash | Health check in `is_available()`; graceful degradation |
| Tool binary not found | `find_binary()` checks multiple paths; clear error message |
| Malformed LLM tool dispatch JSON | Regex extraction fallback; default to "none" |
| Memory pipeline blocks chat | All ingestion runs in `tokio::spawn` (background) |
| Duplicate data from same PDF/search | SHA-256 dedup for PDFs (meta_search handles this); upsert for facts |
