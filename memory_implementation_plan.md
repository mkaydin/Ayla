# Memory System — Implementation Plan

## Docker-Based Architecture

```
┌─────────────────────────────────────────────────┐
│                  Tauri App (Rust)               │
│  ┌───────────────────────────────────────────┐  │
│  │           Memory Manager                  │  │
│  │  ┌─────────┐ ┌──────┐ ┌───────┐         │  │
│  │  │ Working │ │Graph │ │Retriev│         │  │
│  │  │ Memory  │ │Petgr │ │ al    │         │  │
│  │  └─────────┘ └──────┘ └───────┘         │  │
│  └──────────────┬──────────┬────────────────┘  │
│                 │          │                    │
└─────────────────┼──────────┼────────────────────┘
                  │          │
        ┌─────────▼──┐  ┌───▼──────────┐
        │ SurrealDB  │  │    Qdrant     │
        │ :8000      │  │    :6333/6334 │
        │ (struct)   │  │   (vectors)   │
        └────────────┘  └───────────────┘
                  │          │
        ┌─────────▼──────────▼───────────┐
        │  Ollama (localhost:11434)      │
        │  • gemma4:e4b   (extraction)   │
        │  • qwen3-embedding:0.6b (emb)  │
        └────────────────────────────────┘
```

All models already pulled:
- `gemma4:e4b` (9.6 GB) — LLM inference + extraction
- `qwen3-embedding:0.6b` (639 MB) — embedding generation

---

## Phase 1 — Docker Infrastructure

### File: `docker-compose.yml` (project root)

Create a single `docker-compose.yml` that starts SurrealDB and Qdrant:

```yaml
version: "3.9"

services:
  surrealdb:
    image: surrealdb/surrealdb:latest
    container_name: alice-surrealdb
    ports:
      - "8000:8000"
    volumes:
      - ./data/surrealdb:/data
    command: start --log trace --user root --pass root file:/data/memory.db
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "surreal", "is-ready", "--conn", "http://localhost:8000"]
      interval: 10s
      retries: 5

  qdrant:
    image: qdrant/qdrant:latest
    container_name: alice-qdrant
    ports:
      - "6333:6333"
      - "6334:6334"
    volumes:
      - ./data/qdrant:/qdrant/storage
    restart: unless-stopped
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:6333/health"]
      interval: 10s
      retries: 5
```

### Start / Stop

```bash
docker compose up -d        # start both
docker compose down          # stop both
docker compose logs -f       # tail logs
```

### .gitignore additions

```
data/
```

---

## Phase 2 — Rust Crate Dependencies

Add to `src-tauri/Cargo.toml`:

```toml
# --- Memory System ---
surrealdb = { version = "2", features = ["protocol-http", "rustls"] }
qdrant-client = "1"
petgraph = { version = "0.6", features = ["serde-1"] }
tokio = { version = "1", features = ["full"] }

# --- Embedding HTTP ---
reqwest = { version = "0.12", features = ["json"] }

# --- Utilities ---
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
```

---

## Phase 3 — Rust Module Layout

```
src-tauri/src/
├── main.rs
├── lib.rs
├── tts.rs
│
├── memory/                      # NEW — memory subsystem
│   ├── mod.rs                   # MemoryManager orchestrator + MemoryStore trait
│   ├── config.rs                # Connection URLs, collection names
│   ├── working.rs               # Layer 1 — Working memory (VecDeque in RAM)
│   ├── conversation.rs          # Layer 2 — Conversation CRUD (SurrealDB)
│   ├── episodic.rs              # Layer 3 — Episode store (SurrealDB + Qdrant)
│   ├── semantic.rs              # Layer 4 — Facts/preferences (SurrealDB)
│   ├── graph.rs                 # Layer 5 — Petgraph knowledge graph + sync
│   ├── embeddings.rs            # Embedding generator (Ollama qwen3-embedding)
│   ├── extraction.rs            # LLM entity/fact/relation extraction
│   ├── summarizer.rs            # LLM conversation summarization
│   ├── consolidator.rs          # Periodic merge/dedup/forget
│   ├── retrieval.rs             # Hybrid retrieval (graph + semantic + vector)
│   ├── context.rs               # ContextBuilder: merge → rank → prompt
│   └── pipeline.rs              # End-to-end pipeline (summarize → extract → embed → route)
│
└── db/                          # NEW — DB client wrappers
    ├── mod.rs
    ├── surreal.rs               # SurrealDB connection + query helpers
    └── qdrant.rs                # Qdrant connection + upsert/search helpers
```

---

## Phase 4 — Implementation Steps (by module)

### 4.1 — `memory/config.rs`

```rust
/// Connection URLs, model names, collection/table names
pub struct MemoryConfig {
    pub surrealdb_url: String,       // "ws://localhost:8000"
    pub surrealdb_ns: String,        // "alice"
    pub surrealdb_db: String,        // "memory"
    pub qdrant_url: String,          // "http://localhost:6334"
    pub ollama_url: String,          // "http://localhost:11434"
    pub embed_model: String,         // "qwen3-embedding:0.6b"
    pub llm_model: String,           // "gemma4:e4b"
    pub embed_dim: usize,            // 1024 (qwen3-embedding dim)
    pub top_k_retrieval: usize,      // 5
}
```

Default config embeds everything needed. No external config file required initially.

### 4.2 — `db/surreal.rs`

SurrealDB client wrapper:

```rust
pub struct SurrealDB {
    db: surrealdb::Surreal<surrealdb::engine::remote::ws::Client>,
}

impl SurrealDB {
    pub async fn connect(url: &str, ns: &str, db: &str, user: &str, pass: &str) -> Result<Self>
    pub async fn create_table(&self, name: &str) -> Result<()>
    pub async fn insert<T: Serialize>(&self, table: &str, record: T) -> Result<()>
    pub async fn query(&self, sql: &str) -> Result<Vec<T>>
}
```

Tables to create on init:
- `conversation` — full message history per session
- `episode` — summarized important events with embedding IDs
- `fact` — extracted facts (subject, predicate, object, confidence)
- `graph_node` — knowledge graph nodes
- `graph_edge` — knowledge graph edges
- `session` — session metadata

### 4.3 — `db/qdrant.rs`

Qdrant client wrapper:

```rust
pub struct QdrantDB {
    client: qdrant_client::Qdrant,
}

impl QdrantDB {
    pub async fn connect(url: &str) -> Result<Self>
    pub async fn ensure_collection(&self, name: &str, dim: usize) -> Result<()>
    pub async fn upsert(&self, collection: &str, points: Vec<PointStruct>) -> Result<()>
    pub async fn search(&self, collection: &str, vector: Vec<f32>, limit: u64) -> Result<Vec<ScoredPoint>>
}
```

Collections:
- `episodes` — episode summary embeddings
- `conversations` — conversation chunk embeddings
- `facts` — fact embeddings for semantic similarity search

### 4.4 — `memory/embeddings.rs`

Ollama embedding client using `qwen3-embedding:0.6b`:

```rust
pub struct Embedder { config: MemoryConfig }

impl Embedder {
    /// Single text → embedding vector
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>>

    /// Batch embed (Ollama API supports batch)
    pub async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>
}
```

Ollama embedding API:
```
POST http://localhost:11434/api/embed
{"model": "qwen3-embedding:0.6b", "input": ["text to embed"]}
```
Returns: `{"embeddings": [[0.123, 0.456, ...]]}`

qwen3-embedding:0.6b produces 1024-dimensional vectors.

### 4.5 — `memory/working.rs`

Layer 1: Ephemeral, in-RAM, no persistence.

```rust
use std::collections::VecDeque;

pub struct WorkingMemory {
    messages: VecDeque<Message>,       // current conversation turns
    active_task: Option<String>,       // what the user is currently doing
    active_entities: Vec<String>,      // entities mentioned in this session
    max_messages: usize,               // 50
    session_id: String,
}

impl WorkingMemory {
    pub fn new(session_id: &str) -> Self
    pub fn add_message(&mut self, role: &str, content: &str)
    pub fn get_context_window(&self) -> Vec<&Message>  // last N messages for LLM
    pub fn clear(&mut self)                             // reset on new session
}
```

### 4.6 — `memory/conversation.rs`

Layer 2: Full conversation storage in SurrealDB.

```rust
#[derive(Serialize, Deserialize)]
struct ConversationRecord {
    session_id: String,
    turn: u32,
    role: String,            // "user" | "assistant"
    content: String,
    timestamp: chrono::DateTime<chrono::Utc>,
    embedding_id: Option<u64>,  // Qdrant point ID if embedded
}

pub struct ConversationStore { db: SurrealDB }

impl ConversationStore {
    pub async fn save_turn(&self, session_id: &str, role: &str, content: &str)
    pub async fn get_session(&self, session_id: &str) -> Vec<ConversationRecord>
    pub async fn get_recent_context(&self, session_id: &str, n: u32) -> String
}
```

### 4.7 — `memory/episodic.rs`

Layer 3: Important events stored in SurrealDB with vectors in Qdrant.

```rust
#[derive(Serialize, Deserialize)]
struct Episode {
    id: String,
    summary: String,
    importance: f32,          // 0.0–1.0
    entities: Vec<String>,
    timestamp: chrono::DateTime<chrono::Utc>,
    qdrant_point_id: u64,     // link to vector in Qdrant
}

pub struct EpisodeStore {
    surrealdb: SurrealDB,
    qdrant: QdrantDB,
    embedder: Embedder,
}

impl EpisodeStore {
    pub async fn store(&self, summary: &str, importance: f32, entities: &[String])
    pub async fn search_similar(&self, query: &str, limit: u64) -> Vec<Episode>
}
```

### 4.8 — `memory/semantic.rs`

Layer 4: Facts about the user stored in SurrealDB.

```rust
#[derive(Serialize, Deserialize)]
struct Fact {
    id: String,
    subject: String,          // "User"
    predicate: String,        // "prefers"
    object: String,           // "Rust"
    confidence: f32,          // 0.0–1.0, increases with repetition
    source: String,           // session_id where extracted
    first_seen: chrono::DateTime<chrono::Utc>,
    last_seen: chrono::DateTime<chrono::Utc>,
    count: u32,               // number of times confirmed
}

pub struct SemanticStore { db: SurrealDB }

impl SemanticStore {
    pub async fn upsert_fact(&self, subject: &str, predicate: &str, object: &str, confidence: f32)
    pub async fn get_facts_about(&self, subject: &str) -> Vec<Fact>
    pub async fn search_facts(&self, query: &str) -> Vec<Fact>  // text match
}
```

### 4.9 — `memory/graph.rs`

Layer 5: In-memory Petgraph + SurrealDB persistence.

```rust
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::algo;

pub struct KnowledgeGraph {
    graph: DiGraph<GraphNode, GraphEdge>,
    dirty_nodes: HashSet<NodeIndex>,    // track modified nodes for batch save
    db: SurrealDB,
    // Map from entity name → NodeIndex for fast lookup
    name_to_idx: HashMap<String, NodeIndex>,
}

#[derive(Clone, Serialize, Deserialize)]
struct GraphNode {
    name: String,                      // entity name
    node_type: NodeType,               // Person | Project | Tool | Concept | ...
    metadata: serde_json::Value,       // flexible extra data
    created: chrono::DateTime<chrono::Utc>,
    updated: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
struct GraphEdge {
    relation: String,                  // "USES", "WORKS_ON", "KNOWS", ...
    confidence: f32,
    created: chrono::DateTime<chrono::Utc>,
}

enum NodeType { Person, Project, Tool, Concept, Document, Other }

impl KnowledgeGraph {
    pub fn new(db: SurrealDB) -> Self

    /// Load entire graph from SurrealDB into memory on startup
    pub async fn load(&mut self) -> Result<()>

    /// Add or update a node. Returns NodeIndex.
    pub fn upsert_node(&mut self, name: &str, node_type: NodeType) -> NodeIndex

    /// Add or update an edge between two nodes.
    pub fn upsert_edge(&mut self, from: NodeIndex, to: NodeIndex, relation: &str, confidence: f32)

    /// BFS from a node up to `depth` hops. Returns related entity names.
    pub fn neighbors(&self, start: &str, depth: u32) -> Vec<String>

    /// Shortest path between two entity nodes (for context linking).
    pub fn shortest_path(&self, from: &str, to: &str) -> Option<Vec<String>>

    /// Periodically flush dirty nodes/edges to SurrealDB.
    pub async fn sync(&mut self) -> Result<()>
}
```

### 4.10 — `memory/extraction.rs`

LLM-based extraction using `gemma4:e4b` via Ollama. Called after each conversation turn.

Three extraction functions, each using a tailored prompt:

```rust
pub struct Extractor { config: MemoryConfig }

impl Extractor {
    /// Extract named entities from text → Vec<String>
    pub async fn extract_entities(&self, text: &str) -> Vec<String>

    /// Extract facts (subject, predicate, object, confidence) → Vec<(String, String, String, f32)>
    pub async fn extract_facts(&self, text: &str) -> Vec<(String, String, String, f32)>

    /// Extract relationships between entities → Vec<(from, relation, to)>
    pub async fn extract_relations(&self, text: &str) -> Vec<(String, String, String)>
}
```

**Entity extraction prompt:**

```
Extract all named entities from this conversation. Return a JSON array of strings.
Only include: person names, project names, tools, technologies, programming languages,
file names, and significant concepts.

Conversation:
{text}

Output ONLY a JSON array. Example: ["Rust", "Tauri", "Whisper", "Raspberry Pi"]
```

**Fact extraction prompt:**

```
Extract facts from this conversation. Each fact is (subject, predicate, object, confidence).
Confidence: 0.0 (uncertain) to 1.0 (certain).
Return ONLY a JSON array of [subject, predicate, object, confidence].

Conversation:
{text}

Example output:
[["User", "prefers", "Rust", 0.9], ["User", "uses", "Sherpa-ONNX", 1.0]]
```

**Relation extraction prompt:**

```
Extract relationships between entities from this conversation.
Return ONLY a JSON array of [from_entity, relation_type, to_entity].

Conversation:
{text}

Example output:
[["User", "USES", "Sherpa-ONNX"], ["Sherpa-ONNX", "SUPPORTS", "Whisper"]]
```

### 4.11 — `memory/summarizer.rs`

LLM-based conversation summarization. Runs when context window is full or session ends.

```rust
pub struct Summarizer { config: MemoryConfig }

impl Summarizer {
    /// Summarize a conversation turn pair (user + assistant) into 1-2 sentences.
    pub async fn summarize_turn(&self, user_msg: &str, assistant_msg: &str) -> String

    /// Summarize a full session for episodic memory storage.
    pub async fn summarize_session(&self, messages: &[String]) -> String

    /// Estimate importance of a turn (0.0–1.0).
    pub async fn score_importance(&self, user_msg: &str, assistant_msg: &str) -> f32
}
```

**Summarize prompt:**

```
Summarize this conversation exchange in 1-2 sentences. Keep it factual.
Only output the summary, nothing else.

User: {user_msg}
Assistant: {assistant_msg}

Summary:
```

**Importance scoring prompt:**

```
Rate the importance of this conversation from 0.0 (trivial) to 1.0 (critical).
Consider: explicit "remember" requests, personal preferences, project decisions,
emotional significance, repeated topics.

Return ONLY a number like 0.7

User: {user_msg}
Assistant: {assistant_msg}

Importance:
```

### 4.12 — `memory/consolidator.rs`

Periodic memory maintenance. Runs as a background task every N conversations.

```rust
pub struct Consolidator {
    semantic: SemanticStore,
    graph: KnowledgeGraph,
    db: SurrealDB,
}

impl Consolidator {
    /// Merge duplicate or similar facts (e.g., "likes Rust" + "prefers Rust" → "strongly prefers Rust").
    pub async fn merge_facts(&mut self) -> Result<()>

    /// Remove facts below confidence threshold that haven't been confirmed recently.
    pub async fn prune_low_confidence(&mut self, threshold: f32, max_age: Duration) -> Result<()>

    /// Increase confidence for facts that appear frequently.
    pub async fn reinforce_repeated_facts(&mut self) -> Result<()>

    /// Run all consolidation steps.
    pub async fn run(&mut self) -> Result<()>
}
```

### 4.13 — `memory/retrieval.rs`

Hybrid retrieval: runs on every user message to inject relevant memories into the system prompt.

```rust
pub struct HybridRetrieval {
    graph: KnowledgeGraph,
    semantic: SemanticStore,
    episode: EpisodeStore,
    embedder: Embedder,
    qdrant: QdrantDB,
}

#[derive(Serialize)]
pub struct RetrievedContext {
    pub related_facts: Vec<String>,      // facts from semantic store
    pub related_entities: Vec<String>,   // graph neighbors
    pub related_episodes: Vec<String>,   // semantically similar past episodes
    pub recent_conversation: String,     // last few turns (already in working memory)
}

impl HybridRetrieval {
    /// Main entry: given user query, return enriched context for the LLM.
    pub async fn retrieve(&self, query: &str, session_id: &str) -> Result<RetrievedContext>

    // Internal methods
    async fn semantic_search(&self, query_embedding: &[f32]) -> Vec<String>
    async fn graph_search(&self, entities: &[String]) -> Vec<String>
    async fn fact_search(&self, entities: &[String]) -> Vec<String>
}
```

Retrieval flow:
```
User Query
    │
    ├──► Embed query (qwen3-embedding)
    │       ├──► Qdrant search (episodes + conversation chunks) → top episodes
    │       └──► Semantic fact search → matching facts
    │
    ├──► Extract entities from query (lightweight regex / keyword)
    │       └──► Graph neighbors (Petgraph BFS) → related entities
    │
    └──► Merge + deduplicate + rank → RetrievedContext
```

### 4.14 — `memory/context.rs`

Convert retrieved memories into injectable prompt prefix.

```rust
pub struct ContextBuilder;

impl ContextBuilder {
    /// Build the memory-augmented system prompt.
    pub fn build_system_prompt(
        base_prompt: &str,              // Alice's character prompt
        context: &RetrievedContext,
        working_memory: &WorkingMemory,
    ) -> String {
        let mut prompt = String::new();

        // 1. User profile (from semantic facts)
        if !context.related_facts.is_empty() {
            prompt.push_str("## About the User\n");
            for fact in &context.related_facts {
                prompt.push_str(&format!("- {}\n", fact));
            }
            prompt.push('\n');
        }

        // 2. Relevant past episodes
        if !context.related_episodes.is_empty() {
            prompt.push_str("## Relevant Past Conversations\n");
            for ep in &context.related_episodes {
                prompt.push_str(&format!("- {}\n", ep));
            }
            prompt.push('\n');
        }

        // 3. Related knowledge
        if !context.related_entities.is_empty() {
            prompt.push_str("## Related Knowledge\n");
            for e in &context.related_entities {
                prompt.push_str(&format!("- {}\n", e));
            }
            prompt.push('\n');
        }

        // 4. Alice's character prompt (the main system prompt)
        prompt.push_str("---\n\n");
        prompt.push_str(base_prompt);

        prompt
    }
}
```

### 4.15 — `memory/pipeline.rs`

End-to-end memory pipeline. Runs **after** each complete assistant response.

```rust
pub struct MemoryPipeline {
    working: WorkingMemory,
    conversation: ConversationStore,
    episode: EpisodeStore,
    semantic: SemanticStore,
    graph: KnowledgeGraph,
    summarizer: Summarizer,
    extractor: Extractor,
    embedder: Embedder,
}

impl MemoryPipeline {
    /// Called after assistant finishes responding.
    /// 1. Save to conversation store
    /// 2. Summarize the turn
    /// 3. Extract entities → update graph
    /// 4. Extract facts → update semantic store
    /// 5. Score importance → if high, create episodic memory
    /// 6. Embed and store vectors for retrieval
    pub async fn process_turn(
        &mut self,
        user_msg: &str,
        assistant_msg: &str,
    ) -> Result<()> {
        // Step 1: Save raw conversation
        self.conversation.save_turn(&self.working.session_id, "user", user_msg).await?;
        self.conversation.save_turn(&self.working.session_id, "assistant", assistant_msg).await?;
        self.working.add_message("user", user_msg);
        self.working.add_message("assistant", assistant_msg);

        // Step 2: Summarize
        let summary = self.summarizer.summarize_turn(user_msg, assistant_msg).await?;

        // Step 3: Extract entities → graph
        let entities = self.extractor.extract_entities(&format!("{}\n{}", user_msg, assistant_msg)).await?;
        for entity in &entities {
            self.graph.upsert_node(entity, NodeType::Concept);
        }

        // Step 4: Extract facts → semantic
        let facts = self.extractor.extract_facts(&format!("{}\n{}", user_msg, assistant_msg)).await?;
        for (subj, pred, obj, conf) in facts {
            self.semantic.upsert_fact(&subj, &pred, &obj, conf).await?;
        }

        // Step 5: Extract relations → graph edges
        let relations = self.extractor.extract_relations(&format!("{}\n{}", user_msg, assistant_msg)).await?;
        for (from, rel, to) in relations {
            let fi = self.graph.upsert_node(&from, NodeType::Concept);
            let ti = self.graph.upsert_node(&to, NodeType::Concept);
            self.graph.upsert_edge(fi, ti, &rel, 0.8);
        }

        // Step 6: Score importance → episodic memory if significant
        let importance = self.summarizer.score_importance(user_msg, assistant_msg).await?;
        if importance > 0.4 {
            let embedding = self.embedder.embed(&summary).await?;
            // Store in Qdrant
            let point_id = self.qdrant.upsert("episodes", embedding, &summary /*payload*/).await?;
            self.episode.store(&summary, importance, &entities).await?;
        }

        Ok(())
    }

    /// Called when conversation ends — full session summary + graph sync.
    pub async fn end_session(&mut self) -> Result<()>
}
```

### 4.16 — `memory/mod.rs`

The top-level MemoryManager — the single entry point exposed to the rest of the app.

```rust
pub mod config;
pub mod working;
pub mod conversation;
pub mod episodic;
pub mod semantic;
pub mod graph;
pub mod embeddings;
pub mod extraction;
pub mod summarizer;
pub mod consolidator;
pub mod retrieval;
pub mod context;
pub mod pipeline;

use config::MemoryConfig;

pub trait MemoryStore {
    async fn enrich_context(&self, query: &str, base_prompt: &str) -> Result<String>;
    async fn process_turn(&self, user_msg: &str, assistant_msg: &str) -> Result<()>;
    async fn end_session(&self) -> Result<()>;
}

pub struct MemoryManager {
    config: MemoryConfig,
    pipeline: pipeline::MemoryPipeline,
    retrieval: retrieval::HybridRetrieval,
    context_builder: context::ContextBuilder,
}

impl MemoryManager {
    /// Initialize: connect SurrealDB + Qdrant, load graph, ensure tables/collections.
    pub async fn new(config: MemoryConfig) -> Result<Self>

    /// Before LLM call: enrich system prompt with relevant memories.
    pub async fn enrich_context(&self, user_query: &str, base_system_prompt: &str) -> Result<String>

    /// After LLM response: process the turn through the memory pipeline.
    pub async fn process_turn(&self, user_msg: &str, assistant_msg: &str) -> Result<()>
}
```

---

## Phase 5 — Tauri Backend Integration

### 5.1 — `src-tauri/src/lib.rs` changes

```rust
mod tts;
mod memory;
mod db;

use tts::TtsState;
use memory::MemoryManager;
use memory::config::MemoryConfig;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(TtsState {
            engine: std::sync::Mutex::new(tts::TtsEngine::new()),
        })
        .setup(|app| {
            // Init memory system
            let config = MemoryConfig {
                surrealdb_url: "ws://localhost:8000".into(),
                surrealdb_ns: "alice".into(),
                surrealdb_db: "memory".into(),
                qdrant_url: "http://localhost:6334".into(),
                ollama_url: "http://localhost:11434".into(),
                embed_model: "qwen3-embedding:0.6b".into(),
                llm_model: "gemma4:e4b".into(),
                embed_dim: 1024,
                top_k_retrieval: 5,
            };

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let manager = MemoryManager::new(config).await
                    .expect("Failed to initialize memory system");
                handle.manage(manager);
            });

            if cfg!(debug_assertions) { /* logger setup */ }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tts::generate_tts,
            memory_commands::enrich_prompt,
            memory_commands::process_turn,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 5.2 — `src-tauri/src/memory/commands.rs` (new file)

Tauri IPC commands exposed to the JS frontend:

```rust
use tauri::State;
use crate::memory::MemoryManager;

/// Called by frontend BEFORE sending to Ollama.
/// Returns the enriched system prompt with memory context injected.
#[tauri::command]
pub async fn enrich_prompt(
    manager: State<'_, MemoryManager>,
    user_query: String,
    base_prompt: String,
) -> Result<String, String> {
    manager.enrich_context(&user_query, &base_prompt)
        .await
        .map_err(|e| e.to_string())
}

/// Called by frontend AFTER assistant finishes responding.
#[tauri::command]
pub async fn process_turn(
    manager: State<'_, MemoryManager>,
    user_msg: String,
    assistant_msg: String,
) -> Result<(), String> {
    manager.process_turn(&user_msg, &assistant_msg)
        .await
        .map_err(|e| e.to_string())
}
```

### 5.3 — Frontend Integration (`src/main.js`)

Two small changes to the chat pipeline in `chatSendMessage()`:

```javascript
// BEFORE: Build messages array
// const msgs = [chatHistory[0], ...chatHistory.slice(1).slice(-20)];

// AFTER: Enrich system prompt with memories
const ipc = window.__TAURI_INTERNALS__;
const invokeFn = ipc?.invoke || ipc?.ipc?.invoke || ipc?.core?.invoke;

let systemPrompt = chatHistory[0].content;
if (invokeFn) {
  try {
    const enriched = await invokeFn('enrich_prompt', {
      userQuery: query,
      basePrompt: systemPrompt,
    });
    systemPrompt = enriched;
  } catch(e) { /* fall back to base prompt */ }
}

const msgs = [
  { role: 'system', content: systemPrompt },
  ...chatHistory.slice(1).slice(-20)
];

// ... (send to Ollama as before)

// AFTER response is complete (in the finally block or after streaming):
if (invokeFn && fullResponse) {
  invokeFn('process_turn', {
    userMsg: query,
    assistantMsg: fullResponse,
  }).catch(() => {});
}
```

---

## Phase 6 — Startup Sequence

1. `docker compose up -d` → SurrealDB + Qdrant start
2. Ensure Ollama is running: `ollama serve`
3. `npm run tauri dev` → Tauri app starts
4. On app startup:
   - Connect to SurrealDB (WS)
   - Connect to Qdrant (gRPC)
   - Create tables/collections if missing
   - Load knowledge graph from SurrealDB into Petgraph (RAM)
5. On first chat message → memory system kicks in

---

## Phase 7 — Edge Cases & Error Handling

| Scenario | Handling |
|---|---|
| SurrealDB unavailable | Retry 3x with backoff; fall back to working memory only |
| Qdrant unavailable | Skip vector search; use graph + semantic only |
| Ollama embedding fails | Skip embedding; store facts without vectors |
| LLM extraction fails | Skip extraction for this turn |
| Graph too large for RAM (>10k nodes) | Partial load with LRU eviction to SurrealDB |
| Duplicate entities | Merge nodes by name (case-insensitive) |
| Empty query | Skip retrieval, return base prompt |

---

## Phase 8 — Testing Strategy

| Test | How |
|---|---|
| Docker services start | `docker compose up -d && curl localhost:8000/health && curl localhost:6333/health` |
| SurrealDB connect + CRUD | Rust unit test: connect, create table, insert, query |
| Qdrant connect + search | Rust unit test: connect, create collection, upsert, search |
| Embedding generation | Rust unit test: embed "hello world" → expect 1024-dim vector |
| Entity extraction | Rust unit test: extract entities from sample text |
| Fact extraction | Rust unit test: extract facts from sample text |
| Full pipeline | Rust integration test: process a turn → verify stores |
| Enriched prompt | E2E: send query → verify prompt contains memories |

---

## Summary — File Manifest

```
NEW FILES:
├── docker-compose.yml                  # SurrealDB + Qdrant containers
├── src-tauri/src/memory/
│   ├── mod.rs                          # MemoryManager + MemoryStore trait
│   ├── config.rs                       # MemoryConfig defaults
│   ├── commands.rs                     # Tauri IPC commands
│   ├── working.rs                      # Layer 1: Working memory
│   ├── conversation.rs                 # Layer 2: Conversation store
│   ├── episodic.rs                     # Layer 3: Episodic memory
│   ├── semantic.rs                     # Layer 4: Semantic facts
│   ├── graph.rs                        # Layer 5: Petgraph knowledge graph
│   ├── embeddings.rs                   # Ollama qwen3-embedding client
│   ├── extraction.rs                   # LLM entity/fact/relation extraction
│   ├── summarizer.rs                   # LLM conversation summarizer
│   ├── consolidator.rs                 # Periodic memory maintenance
│   ├── retrieval.rs                    # Hybrid retrieval engine
│   ├── context.rs                      # Prompt builder from memories
│   └── pipeline.rs                     # End-to-end memory pipeline
├── src-tauri/src/db/
│   ├── mod.rs
│   ├── surreal.rs                      # SurrealDB client wrapper
│   └── qdrant.rs                       # Qdrant client wrapper

MODIFIED FILES:
├── src-tauri/Cargo.toml                # Add surrealdb, qdrant-client, petgraph, reqwest, etc.
├── src-tauri/src/lib.rs                # Init memory manager, manage state, register commands
├── src/main.js                         # enrich_prompt before send, process_turn after response
├── .gitignore                          # Add data/
```

---

## Implementation Order

```
1. docker-compose.yml           (15 min)   ← infrastructure ready
2. Cargo.toml deps              (5 min)    ← dependencies added
3. memory/config.rs             (10 min)   ← config struct
4. db/surreal.rs                (30 min)   ← SurrealDB connect + CRUD
5. db/qdrant.rs                 (30 min)   ← Qdrant connect + search
6. memory/embeddings.rs         (20 min)   ← Ollama embed API
7. memory/working.rs            (15 min)   ← simple in-memory queue
8. memory/conversation.rs       (20 min)   ← SurrealDB conversation CRUD
9. memory/semantic.rs           (20 min)   ← facts table CRUD
10. memory/graph.rs             (45 min)   ← Petgraph + SurrealDB sync
11. memory/extraction.rs        (30 min)   ← LLM extraction prompts
12. memory/summarizer.rs        (20 min)   ← LLM summarization
13. memory/episodic.rs          (25 min)   ← episode store + vectors
14. memory/retrieval.rs         (30 min)   ← hybrid retrieval
15. memory/context.rs           (15 min)   ← prompt building
16. memory/pipeline.rs          (30 min)   ← end-to-end pipeline
17. memory/mod.rs               (20 min)   ← MemoryManager orchestrator
18. memory/commands.rs          (15 min)   ← Tauri IPC
19. lib.rs integration          (15 min)   ← wire into Tauri
20. main.js integration         (15 min)   ← wire into frontend
21. memory/consolidator.rs      (20 min)   ← background maintenance
─────────────────────────────────────────
Total estimated: ~7 hours
```
