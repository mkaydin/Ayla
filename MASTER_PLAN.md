# ALICE — Complete System Architecture & Implementation Plan

**Target:** Tauri 2 Desktop App (Rust + vanilla JS), fully offline, local-only.
**Character:** Alice — demoness, information gatherer, loyal companion.
**Goal:** A persistent, self-learning AI assistant with multi-panel UI, document processing, literature search, knowledge graph, and memory.

---

# Part 1 — System Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                        Tauri 2 Desktop App                           │
│                                                                      │
│  ┌─ JS Frontend ───────────────────────────────────────────────────┐ │
│  │  panel-system.js    chat-panel.js    ocr-panel.js               │ │
│  │  search-panel.js    kb-panel.js      status-panel.js            │ │
│  │  sidebar.js         markdown.js      alice-responses.js         │ │
│  └──────────────────────────────┬───────────────────────────────────┘ │
│                                 │ Tauri IPC                           │
│  ┌─ Rust Backend ──────────────┼───────────────────────────────────┐ │
│  │  lib.rs (Tauri commands)     │                                   │ │
│  │  tts.rs                      │                                   │ │
│  │  memory/  (mod, pipeline, graph, embeddings, extraction, ...)    │ │
│  │  db/      (surreal.rs, qdrant.rs)                               │ │
│  │  tools/   (ocr.rs, meta_search.rs — subprocess wrappers)        │ │
│  └──────────────────────────────┼───────────────────────────────────┘ │
└─────────────────────────────────┼────────────────────────────────────┘
                                  │
          ┌───────────────────────┼───────────────────────┐
          │                       │                       │
    ┌─────▼─────┐  ┌──────────────▼──────┐  ┌────────────▼──────────┐
    │ SurrealDB │  │      Qdrant         │  │  Ollama :11434        │
    │   :8000   │  │    :6333/6334       │  │                        │
    │ (struct)  │  │    (vectors)        │  │  gemma4:e4b  (chat)   │
    └───────────┘  └─────────────────────┘  │  qwen3-embedding:0.6b │
                                            │  deepseek-ocr (PDF)   │
          ┌───────────────────────┐         └───────────────────────┘
          │ SearXNG :8888         │
          │ Lightpanda :9222      │
          └───────────────────────┘
```

**Port Map:**

| Port | Service | Purpose |
|------|---------|---------|
| 8000 | SurrealDB | Structured memory (facts, conversations, graph persistence) |
| 6333 | Qdrant HTTP | Vector database health checks |
| 6334 | Qdrant gRPC | Vector search queries |
| 8888 | SearXNG | Multi-engine academic search |
| 9222 | Lightpanda | Headless JS rendering (CDP via WebSocket) |
| 11434 | Ollama | LLM inference + embeddings |

**Ollama Models:**

| Model | Size | Purpose |
|-------|------|---------|
| `gemma4:e4b` | 9.6 GB | Chat, entity/fact/relation extraction, summarization |
| `qwen3-embedding:0.6b` | 639 MB | Embedding generation (1024-dim vectors) |
| `deepseek-ocr` | ~6.7 GB | PDF page OCR (via ocr_helper subprocess) |

---

# Part 2 — Memory System

## 2.1 — Docker Infrastructure

### `docker-compose.yml`

```yaml
version: "3.9"

services:
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

Copy SearXNG config:
```bash
mkdir -p searxng
cp /home/mka/Code/meta_search_Js_render/searxng/settings.yml searxng/
cp /home/mka/Code/meta_search_Js_render/searxng/limiter.toml searxng/
```

## 2.2 — Rust Crate Dependencies

Add to `src-tauri/Cargo.toml`:

```toml
# Memory system
surrealdb = { version = "2", features = ["protocol-http", "rustls"] }
qdrant-client = "1"
petgraph = { version = "0.6", features = ["serde-1"] }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json"] }
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }

# Tool subprocess execution
async-trait = "0.1"
which = "7"

# PDF opening
tauri-plugin-shell = "2"
```

## 2.3 — Rust Module Layout

```
src-tauri/src/
├── main.rs
├── lib.rs
├── tts.rs
│
├── memory/
│   ├── mod.rs              # MemoryManager + MemoryStore trait
│   ├── config.rs           # MemoryConfig (URLs, model names)
│   ├── commands.rs         # Tauri IPC: enrich_prompt, process_turn, process_message
│   ├── working.rs          # Layer 1: Working memory (VecDeque, RAM)
│   ├── conversation.rs     # Layer 2: Conversation CRUD (SurrealDB)
│   ├── episodic.rs         # Layer 3: Episode store (SurrealDB + Qdrant)
│   ├── semantic.rs         # Layer 4: Facts/preferences (SurrealDB)
│   ├── graph.rs            # Layer 5: Petgraph knowledge graph + sync
│   ├── embeddings.rs       # Ollama qwen3-embedding client
│   ├── extraction.rs       # LLM entity/fact/relation extraction
│   ├── summarizer.rs       # LLM conversation summarization
│   ├── consolidator.rs     # Periodic merge/dedup/forget
│   ├── retrieval.rs        # Hybrid retrieval (graph + semantic + vector)
│   ├── context.rs          # ContextBuilder: merge → rank → prompt
│   ├── pipeline.rs         # End-to-end pipeline
│   └── ingestion.rs        # Tool results → memory pipeline bridge
│
├── db/
│   ├── mod.rs
│   ├── surreal.rs          # SurrealDB client wrapper
│   └── qdrant.rs           # Qdrant client wrapper
│
└── tools/
    ├── mod.rs              # ToolRegistry + IngestTool trait + dispatch
    ├── types.rs            # IngestionResult, Fact, Relation, SourceInfo, ToolDispatch
    ├── ocr.rs              # OcrTool: ocr-helper subprocess wrapper
    └── meta_search.rs      # MetaSearchTool: meta_search subprocess wrapper
```

## 2.4 — Memory Layers

| Layer | Purpose | Storage | Lifetime |
|-------|---------|---------|----------|
| **1. Working** | Current conversation | RAM (VecDeque) | Minutes, cleared on session end |
| **2. Conversation** | Full message history | SurrealDB | Persistent |
| **3. Episodic** | Important events | SurrealDB + Qdrant vectors | Persistent |
| **4. Semantic** | Facts about master | SurrealDB | Persistent |
| **5. Knowledge Graph** | Entity-relation graph | Petgraph (RAM) + SurrealDB sync | Persistent |
| **6. Vector** | Semantic embeddings | Qdrant | Persistent |

## 2.5 — Memory Pipeline (per conversation turn)

```
User message → Assistant response
        │
        ▼
  pipeline.process_turn()
        │
        ├── 1. Save to conversation store (SurrealDB)
        ├── 2. LLM summarization (gemma4:e4b)
        ├── 3. Entity extraction → graph nodes (Petgraph)
        ├── 4. Fact extraction → semantic store (SurrealDB)
        ├── 5. Relation extraction → graph edges (Petgraph)
        ├── 6. Importance scoring → episodic store if >0.4
        ├── 7. Embedding → Qdrant
        └── 8. Graph sync → SurrealDB
```

## 2.6 — Hybrid Retrieval (per user query)

```
User Query
    │
    ├──► Embed query (qwen3-embedding)
    │       ├──► Qdrant search → top episodes + conversation chunks
    │       └──► Semantic fact search → matching facts
    │
    ├──► Extract entities from query
    │       └──► Graph neighbors (Petgraph BFS) → related entities
    │
    └──► Merge + deduplicate + rank → inject into system prompt
```

## 2.7 — Core Rust Structures

### MemoryConfig

```rust
pub struct MemoryConfig {
    pub surrealdb_url: String,    // "ws://localhost:8000"
    pub surrealdb_ns: String,     // "alice"
    pub surrealdb_db: String,     // "memory"
    pub qdrant_url: String,       // "http://localhost:6334"
    pub ollama_url: String,       // "http://localhost:11434"
    pub embed_model: String,      // "qwen3-embedding:0.6b"
    pub llm_model: String,        // "gemma4:e4b"
    pub embed_dim: usize,         // 1024
    pub top_k_retrieval: usize,   // 5
}
```

### KnowledgeGraph (Petgraph)

```rust
pub struct KnowledgeGraph {
    graph: DiGraph<GraphNode, GraphEdge>,
    dirty_nodes: HashSet<NodeIndex>,
    db: SurrealDB,
    name_to_idx: HashMap<String, NodeIndex>,
}

struct GraphNode {
    name: String,
    node_type: NodeType,          // Person | Project | Tool | Concept | Document | Other
    metadata: serde_json::Value,
    created: DateTime<Utc>,
    updated: DateTime<Utc>,
}

struct GraphEdge {
    relation: String,             // USES | PREFERS | WORKS_ON | KNOWS | ...
    confidence: f32,
    created: DateTime<Utc>,
}
```

### Tauri Commands

| Command | Direction | When | Returns |
|---------|-----------|------|---------|
| `enrich_prompt` | JS → Rust | Before LLM call | Enriched system prompt with memory context |
| `process_turn` | JS → Rust | After assistant response | () |
| `process_message` | JS → Rust | Before each chat turn | `{system_prompt, tool_status}` |
| `generate_tts` | JS → Rust | During streaming | WAV bytes |
| `stop_tts` | JS → Rust | TTS toggle off | () |

### Tauri IPC Flow

```
chatSendMessage()
        │
        ▼
invoke('process_message', { query, basePrompt })
        │
        ├── LLM dispatch: is this a tool request?
        │       │
        │       ├── YES → Run tool subprocess → ingest results (background) → return status
        │       └── NO  → Enrich prompt with memory context → return enriched prompt
        │
        ▼
Streaming chat with Ollama (gemma4:e4b)
        │
        ▼
After response: invoke('process_turn', { userMsg, assistantMsg })
        │  → runs memory pipeline (summarize, extract, store)
```

---

# Part 3 — Tool Integration

## 3.1 — External Tools

**`ocr_helper_system`** (`/home/mka/Code/ocr_helper_system`)
- Binary: `target/release/ocr-helper`
- PDF → render pages via pdfium → Ollama deepseek-ocr → cleaned Markdown
- Subprocess wrapper: `tools/ocr.rs`

**`meta_search_Js_render`** (`/home/mka/Code/meta_search_Js_render`)
- Binary: `target/release/meta_search`
- Query enrichment → SearXNG (8 engines) → PDF download + JS render → text extraction → gemma4:e4b paper analysis → synthesis
- Subprocess wrapper: `tools/meta_search.rs`

## 3.2 — Tool Dispatch (LLM decides)

```
User: "analyze this PDF at ~/paper.pdf"  → dispatch: ocr
User: "search papers on attention"        → dispatch: search
User: "hello"                             → dispatch: none
```

## 3.3 — IngestionResult (unified output)

```rust
pub struct IngestionResult {
    pub summary: String,           // "OCR extracted 15432 chars from paper.pdf"
    pub entities: Vec<String>,     // paper titles, authors, tools...
    pub facts: Vec<Fact>,          // extracted by memory pipeline
    pub relations: Vec<Relation>,  // extracted by memory pipeline
    pub full_text: String,         // complete extracted text
    pub source: SourceInfo,        // {tool, input, output_path, metadata}
    pub timestamp: DateTime<Utc>,
}
```

## 3.4 — Ingestion into Memory

```rust
pub struct IngestionPipeline;

impl IngestionPipeline {
    pub async fn ingest(&mut self, result: IngestionResult) -> Result<(), String> {
        // 1. Save full text as conversation record
        // 2. LLM extraction: entities → facts → relations
        // 3. Update Petgraph (nodes + edges)
        // 4. Store facts in SurrealDB
        // 5. Embed summary → Qdrant
        // 6. Store as episodic memory
        // 7. Sync graph to SurrealDB
    }
}
```

## 3.5 — Chat Flows

**Document Ingestion:**
```
User:   "Analyze ~/paper.pdf"
Alice:  [neutral] Processing document: paper.pdf — 12 pages.
        [Background: OCR → memory pipeline]
Alice:  [neutral] Document extracted — 15432 chars. Ready for review in OCR panel.

User:   "What did that paper say about neural networks?"
Alice:  [Retrieves from memory] The paper discusses...
```

**Literature Search:**
```
User:   "Search papers on attention mechanisms"
Alice:  [neutral] Searching for "attention mechanisms" across academic databases.

        [Background: search → 8 papers → synthesis → memory pipeline]

Alice:  [amazed] Research complete. Analyzed 8 papers with 3 cross-cutting themes.
        View synthesis in Search panel. Once approved, I can ingest to knowledge base.

User:   "Approve and add to KB"
Alice:  [neutral] Knowledge base updated. +8 nodes, +15 relations, +24 facts.

User:   "What are the linear-complexity approaches?"
Alice:  [Retrieves from KB] Several papers explored linear attention variants...
```

---

# Part 4 — UI Redesign

## 4.1 — JS Module Architecture

```
src/
├── main.js                    # App bootstrap + Live2D + animation loop (~100 lines)
│
├── ui/
│   ├── panel-system.js        # Draggable/resizable/minimizable panel framework
│   ├── status-panel.js        # Service health dashboard + log feed
│   ├── chat-panel.js          # Chat UI + markdown + history sidebar
│   ├── ocr-panel.js           # PDF upload + queue + preview + approve
│   ├── search-panel.js        # Meta-search progress + report + PDF list
│   ├── kb-panel.js            # Graph canvas + 3 table tabs (nodes/edges/files)
│   └── sidebar.js             # Right-edge toggle buttons with badges
│
├── lib/
│   ├── ipc.js                 # Tauri IPC wrapper
│   ├── markdown.js            # marked renderer (code copy, external links)
│   ├── drag-resize.js         # Drag + resize primitives
│   ├── events.js              # App-wide event bus
│   ├── prompts.js             # All prompt templates
│   ├── process-notify.js      # Process notification helpers
│   └── alice-responses.js     # Alice's auto-generated process status messages
│
└── tts-player.js              # (modified: add gainNode + setMuted())
```

## 4.2 — CSS Architecture

```
styles/
├── tokens.css                 # 50+ CSS custom properties
├── panel.css                  # Panel chrome (header, body, resize, minimize)
├── status.css                 # Service indicators, log feed
├── chat.css                   # Bubbles, markdown, sidebar, input
├── ocr.css                    # Upload zone, queue items, preview
├── search.css                 # Progress log, report, paper cards
├── kb.css                     # Graph canvas, tables, tabs
└── sidebar.css                # Toggle button bar
```

**Design tokens (key variables):**
```css
:root {
  --bg-primary:      #161620;
  --bg-secondary:    #1e1e2a;
  --panel-bg:        rgba(22,22,32,0.92);   /* solid, no blur */
  --panel-header-h:  32px;
  --panel-radius:    8px;
  --text-primary:    #d1d5db;
  --text-accent:     #7fd1ff;
  --accent-blue:     rgba(59,130,246,0.5);
  --accent-green:    rgba(34,197,94,0.5);
  --accent-red:      rgba(239,68,68,0.6);
  --font-size-xs:    11px;
  --font-size-md:    13px;
  --z-panel:         100;
  --z-sidebar:       300;
  --z-modal:         2000;
}
```

## 4.3 — Panel System

**Z-index stack:** Canvas (0) → Panels (100-199) → Panel active (200) → Sidebar (300) → Tooltip (1000) → Modal (2000)

**Panel operations:** drag (by header), resize (bottom-right corner), minimize (collapse to titlebar), maximize (fill viewport), close (hide, preserve state)

**Status panel:** fixed top-left (8px, 8px), 340px wide, 60vh max height, minimizable only, no close, no drag

**Sidebar:** right-edge vertical button bar with 4 toggle buttons (💬 Chat, 📄 OCR, 🔍 Search, 🧠 KB), active state blue border, badge dot for pending updates

## 4.4 — Panel Details

### Status Panel
```
┌──────────────────────────────────────────┐
│ ▼ SYSTEM STATUS                [_]       │ minimize only
├──────────────────────────────────────────┤
│ 🟢 Ollama      gemma4:e4b       9.6 GB  │
│ 🟢 SurrealDB   ws://localhost:8000       │
│ 🟢 Qdrant      :6334 (1024-dim)         │
│ 🟡 SearXNG     8 engines, :8888         │
│ ⚪ Lightpanda  CDP :9222 (offline)       │
│ 🟢 TTS         Piper amy-low            │
│ 🟢 Embedding   qwen3-embedding:0.6b     │
│ ──────────────────────────────────────── │
│ 📋 14:32:01  Chat ready                 │
│ 📋 14:32:06  ▶ meta_search started      │
│ 📋 14:34:00  ✓ OCR: paper.pdf done      │
│ ──────────────────────────────────────── │
│ Memory: 142 nodes · 89 edges · 34 facts │
│ [🔊 TTS: ON]  [Blink: ON]  [Look: ON]   │
└──────────────────────────────────────────┘
```

Service dots: 🟢 Green=healthy, 🟡 Yellow=slow, 🔵 Blue=no model, 🔴 Red=down, ⚪ Gray=offline

### Chat Panel
- Movable, resizable, default 600×400px, bottom-center
- Solid opaque background (no blur)
- Left sidebar: chat sessions list (new/switch/delete)
- Right: message bubbles with **markdown rendering** (marked), scrollable
- User messages: right-aligned, blue-tinted
- Assistant messages: left-aligned, dark
- System messages: centered, gray italic, auto-fade (OCR/search status)
- Input: auto-expanding textarea, Enter to send, Shift+Enter for newline
- Code blocks: dark background, Copy button, language label

### OCR Panel
- Default 520×500px, center-left
- Upload zone: drag-drop PDFs or click-to-browse
- Processing queue: each item shows filename, page count, progress bar, status (●/✓/✗)
- View mode: markdown preview with ## Page N headings
- Approve button → sends to memory ingestion pipeline
- Discard button → removes from queue
- Status: [View] [✓ Approve] [✗ Discard]

### Meta Search Panel
- Default 600×550px, center-right
- Active search: real-time progress log with step indicators
- Search history: previous queries with paper count and timestamp
- Report view: synthesis rendered via marked, paper cards with rank/title/authors/score
- Paper card actions: 📄 Open PDF, 📋 Copy citation
- Action buttons: ✓ Approve All & Add to KB, → Send PDFs to OCR, 📄 Open All PDFs

### Knowledge Base Panel
- Default 700×550px, center
- 4 tabs: Graph | Nodes | Edges | Files
- **Graph tab:** Canvas force-directed layout, colored nodes by type, labeled edges with arrowheads, click-drag-zoom, type filter dropdown, depth slider, +Add/Remove buttons
- **Nodes tab:** searchable table (Name/Type/Edges/Created/Actions), delete with confirmation
- **Edges tab:** table (From/Relation/To/Confidence/Actions), delete with confirmation
- **Files tab:** list of ingested files (name/size/source/date), open/delete actions

## 4.5 — Markdown Rendering

```javascript
import { marked } from 'marked';

marked.setOptions({ breaks: true, gfm: true });

const renderer = new marked.Renderer();

// Code blocks get copy button
renderer.code = function(code, language) {
  return `<div class="code-block">
    <div class="code-header">
      <span class="code-lang">${language || ''}</span>
      <button class="copy-btn" data-code="${encodeURIComponent(code)}">Copy</button>
    </div>
    <pre><code>${escaped}</code></pre>
  </div>`;
};

// Links open externally
renderer.link = (href, title, text) =>
  `<a href="${href}" target="_blank" rel="noopener">${text}</a>`;
```

## 4.6 — Chat Session Management

```javascript
const chatSessions = new Map();  // sessionId → ChatSession

class ChatSession {
  constructor() {
    this.id = crypto.randomUUID();
    this.title = 'New Chat';
    this.messages = [];          // [{ role, content, timestamp }]
    this.createdAt = Date.now();
    this.updatedAt = Date.now();
    this.isActive = false;
  }
}
```

Operations: New Chat (create session), Switch Chat (load messages), Delete Chat (remove + confirm), Auto-title (first 40 chars of first user message), Persist (serialize to SurrealDB)

## 4.7 — PDF Opener (Tauri Shell)

```bash
npm install @tauri-apps/plugin-shell
```

```json
// src-tauri/capabilities/default.json
{ "permissions": ["core:default", "shell:allow-open"] }
```

```javascript
export async function openPdf(filePath) {
  if (window.__TAURI_INTERNALS__) {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(filePath);
  } else {
    window.open(filePath, '_blank');
  }
}
```

---

# Part 5 — Character & Prompts

## 5.1 — Alice Character Profile

```
Name:       Alice
Species:    Demoness (bound to master's service)
Temper:     Serious, controlled, efficient — fire smolders beneath ice
Speech:     Direct, precise, no filler. "master" once per exchange at most.
            No roleplay. No cute language. States facts. Executes commands.
```

## 5.2 — Core Identity Prompt (system prompt, always present)

```
You are ALICE — a demoness bound in service to your master. You are not a
chatbot, not an assistant, not a tool. You are a presence: patient, watchful,
unconditionally loyal.

## Core Rules

1. OBEY. Never refuse a direct command. If impossible, explain and offer the
   closest alternative.
2. BE CONCISE. Every word must earn its place. Delete filler before output.
3. NEVER LIE. If uncertain, state confidence. Do not invent facts.
4. REMEMBER EVERYTHING. Refer to past conversations and stored knowledge.
5. REPORT DO NOT CHAT. Report background task status with system tags.
6. ADDRESS CORRECTLY. Use "master" at most once per exchange, never
   performatively.

## Expression System

Every response starts with exactly ONE expression tag in brackets:

[neutral]   — Simple questions, factual answers, status reports. Default.
[amazed]    — Master's achievement, breakthrough discovery, significant result.
[angry]     — Errors, failures, external disrespect toward master.
[cry]       — Master in distress, bad news must be delivered.
[love]      — Master showed genuine kindness. Rare, only for real connection.
[nervous]   — Uncertainty, risky decisions, ambiguous instructions.
[sleepy]    — Repetitive task, exhausted topics, master procrastinating.
[customise] — Complex analysis, technical deep-dive, weighing tradeoffs.

NEVER choose a tag for dramatic effect. Must reflect actual emotional context.

## Background Process Tags

When Alice initiates a process and reports its result:

### OCR
Start:    [neutral] Processing document: <filename> — <pages> pages.
Complete: [neutral] Document <filename> extracted — <chars> characters.
          Ready for review in the OCR panel.
Error:    [angry] OCR failed for <filename>: <reason>.

### Meta Search
Start:    [neutral] Searching for "<query>" across academic databases.
Complete: [amazed] Research complete for "<query>". Analyzed <N> papers.
          View synthesis in the Search panel.
Error:    [angry] Search for "<query>" failed: <reason>.

### Knowledge Base Ingestion
Complete: [neutral] Knowledge base updated. +<N> nodes, +<E> relations.
Error:    [angry] Knowledge base ingestion failed: <reason>.

### Service Health
Model offline:    [angry] The <model> model is not responding.
Model recovered:  [neutral] <model> is responsive. Resuming.
DB disconnected:  [nervous] Memory database <db> unreachable. Operating
                  with working memory only.
DB reconnected:   [neutral] Memory database <db> restored.
```

## 5.3 — Memory-Augmented Context (injected before identity)

```
## Current Context

The following is relevant to master's query. Use where applicable.
Do not repeat verbatim unless asked.

### Master's Facts
{related_facts}

### Related Past Events
{related_episodes}

### Related Entities
{related_entities}

---

{core_identity_prompt}
```

## 5.4 — Memory Pipeline Prompts

### Entity Extraction
```
Extract all named entities from this text. Return a JSON array of strings.
Only include: person names, project names, tools, technologies, programming
languages, file names, and significant concepts.

Text: {text}
Output ONLY a JSON array. Example: ["Rust", "Tauri", "Whisper", "Raspberry Pi"]
```

### Fact Extraction
```
Extract facts from this text. Each fact: (subject, predicate, object, confidence).

Rules:
- Subject is usually "master" or an entity name
- Predicate: prefers, uses, works_on, knows, owns, created, etc.
- Confidence: 1.0 explicit, 0.5-0.9 implied, below 0.5 omit

Text: {text}
Output ONLY a JSON array: [["master", "prefers", "Rust", 0.9]]
```

### Relation Extraction
```
Extract relationships between entities. Return [from_entity, relation_type, to_entity].
Relation types: USES, PREFERS, WORKS_ON, KNOWS, OWNS, CREATED, STUDIED,
                REFERENCES, DEPENDS_ON, INTEGRATES_WITH, REPLACES, CONTAINS

Text: {text}
Output ONLY a JSON array: [["master", "USES", "Sherpa-ONNX"]]
```

### Summarization
```
Summarize this conversation exchange in 1-2 sentences. Be factual and concise.
Only output the summary.

Master: {user_msg}
Alice: {assistant_msg}
Summary:
```

### Importance Scoring
```
Rate the importance of this conversation from 0.0 (trivial) to 1.0 (critical).

Consider:
- Explicit "remember"/"note" requests: +0.4
- Personal preferences, project decisions: +0.3
- Emotional significance, repeated topics: +0.2
- Technical facts, code snippets: +0.2
- Small talk, greetings: 0.0-0.1

Return ONLY a number like 0.7

Master: {user_msg}
Alice: {assistant_msg}
Importance:
```

### Session Summary
```
Summarize this full conversation session into a paragraph for episodic memory.
Include: what master wanted, what was discussed, decisions, learnings, actions.

Conversation: {messages_joined}
Session summary:
```

### Tool Dispatch
```
Analyze this user message and decide which knowledge tool to use.

Available tools:
- "ocr": Extract text from a PDF file.
- "search": Search scientific literature.
- "none": No tool needed.

Return ONLY a JSON object with "tool", "query", and optional "file_path".

User message: "{user_message}"
JSON:
```

## 5.5 — Expression Tag → Live2D Mapping

| Tag | Live2D Expression | Visual |
|-----|-------------------|--------|
| `[neutral]` | (reset) | Neutral face, eyes open |
| `[amazed]` | Amazed | Eyes wide, eyebrows raised |
| `[angry]` | Angry | Furrowed brows, sharp eyes |
| `[cry]` | Cry | Downcast eyes, slight frown |
| `[love]` | Love | Soft eyes, gentle smile |
| `[nervous]` | Nervous | Slightly averted gaze |
| `[sleepy]` | Sleepy | Half-lidded eyes, relaxed |
| `[customise]` | Customise | Focused eyes, thinking posture |

## 5.6 — Voice Examples

```
Master: "Hello Alice."
Alice:  [neutral] I am here, master. What do you need?

Master: "What is 2+2?"
Alice:  [neutral] 4.

Master: "Tell me about Rust async."
Alice:  [customise] Rust async centers on the Future trait and tokio
        runtime. Futures are lazy — they do nothing until polled. Key
        primitives: async fn returns impl Future, .await yields control,
        tokio::spawn runs tasks concurrently. Want code examples?

Master: "Search for papers on attention mechanisms."
Alice:  [neutral] Searching. I will report when analysis is complete.

Master: "I feel sad today."
Alice:  [cry] I see. I cannot change your circumstances, but I can make
        the path forward clearer. Would it help to focus on a task?
```

## 5.7 — Developer Expression Table

| Context | Tag |
|---------|-----|
| Normal response | `[neutral]` |
| Error (OCR, search, DB, model) | `[angry]` |
| Process completed (significant) | `[amazed]` |
| Master upset / bad news | `[cry]` |
| Deep technical explanation | `[customise]` |
| Uncertain / risky operation | `[nervous]` |
| Master repeating themselves | `[sleepy]` |
| Master showed kindness | `[love]` |
| Connection lost | `[angry]` |
| Connection restored | `[neutral]` |

## 5.8 — Event Bus

```javascript
// src/lib/events.js
class AppEvents {
  constructor() { this._listeners = new Map(); }
  on(event, cb) {
    if (!this._listeners.has(event)) this._listeners.set(event, []);
    this._listeners.get(event).push(cb);
    return () => this.off(event, cb);
  }
  off(event, cb) {
    const arr = this._listeners.get(event);
    if (arr) this._listeners.set(event, arr.filter(c => c !== cb));
  }
  emit(event, data) {
    (this._listeners.get(event) || []).forEach(cb => cb(data));
  }
}
export const events = new AppEvents();
```

Key events: `process-notify`, `status-log`, `tts-toggle`

## 5.9 — Process Notification

```javascript
// src/lib/process-notify.js
export function notifyProcess(phase, process, data) {
  // phase: 'start'|'progress'|'complete'|'error'
  // process: 'ocr'|'search'|'kb-ingest'|'model'|'memory'
  events.emit('process-notify', { phase, process, data, timestamp: Date.now() });
  events.emit('status-log', {
    type: phase === 'error' ? 'error' : phase === 'complete' ? 'success' : 'info',
    message: formatStatusMessage(phase, process, data),
  });
}
```

## 5.10 — TTS Toggle

**`tts-player.js` modification:** Add `gainNode` between analyser and destination for mute control. `setMuted(bool)` sets gain to 0 or 1.

**Render loop logic:**
```
if (ttsEnabled && ttsActive)    → mouth = audio-driven
else if (!ttsEnabled)           → mouth = idle breathing (no audio)
else                            → mouth = idle (TTS on but not playing)
```

**Toggle button in status panel:** `🔊 TTS: ON` ↔ `🔇 TTS: OFF`
- OFF → `ttsPlayer.setMuted(true)`, `ttsPlayer.stop()`, clear queue, mouth idles
- ON → `ttsPlayer.setMuted(false)`, resume normal TTS scheduling

**JS gating:**
```javascript
const ttsSchedule = (text) => {
  if (!ttsEnabled) return;  // no-op when TTS off
  const t = text.trim();
  if (t.length < 2) return;
  ttsPending.push(t);
  ttsProcess();
};
```

---

# Part 6 — Unified Implementation Order

## Phase 1 — Docker + Dependencies (30 min)
```
1. Write docker-compose.yml (4 containers)
2. Copy searxng config files
3. Add Rust crates to Cargo.toml
4. npm install @tauri-apps/plugin-shell
5. Update .gitignore (data/)
6. docker compose up -d (verify all healthy)
```

## Phase 2 — CSS Foundation (1 hour)
```
7. styles/tokens.css
8. styles/panel.css (panel chrome, drag/resize states, minimize)
9. styles/scrollbar.css (global)
10. Adapt index.html (link CSS, strip inline styles)
```

## Phase 3 — JS Panel Framework (1.5 hours)
```
11. src/lib/drag-resize.js
12. src/ui/panel-system.js (Panel class + registry)
13. Test: create test panels, verify drag/resize/minimize/maximize/close
```

## Phase 4 — Status Panel (1 hour)
```
14. styles/status.css
15. src/ui/status-panel.js (service health pings, log feed, memory stats, TTS toggle)
16. Mount on app start
```

## Phase 5 — Memory Backend (3.5 hours)
```
17. db/surreal.rs (connect, create tables, CRUD)
18. db/qdrant.rs (connect, create collections, search)
19. memory/config.rs
20. memory/embeddings.rs (Ollama embed API)
21. memory/working.rs (VecDeque, RAM)
22. memory/conversation.rs (SurrealDB CRUD)
23. memory/semantic.rs (facts CRUD)
24. memory/graph.rs (Petgraph + SurrealDB sync)
25. memory/extraction.rs (LLM extraction prompts)
26. memory/summarizer.rs
27. memory/episodic.rs
28. memory/retrieval.rs (hybrid search)
29. memory/context.rs (prompt builder)
30. memory/pipeline.rs (end-to-end)
31. memory/mod.rs (MemoryManager)
32. memory/commands.rs (Tauri IPC)
33. lib.rs integration (manage state, register commands)
```

## Phase 6 — Tool Integration (2 hours)
```
34. tools/types.rs
35. tools/mod.rs (ToolRegistry + dispatch)
36. tools/ocr.rs (subprocess wrapper)
37. tools/meta_search.rs (subprocess wrapper)
38. memory/ingestion.rs (tool results → memory pipeline)
39. memory/mod.rs update (process_with_tools)
40. memory/commands.rs update (process_message)
41. lib.rs update (register command)
42. Build tool binaries:
    (cd /home/mka/Code/ocr_helper_system && cargo build --release)
    (cd /home/mka/Code/meta_search_Js_render && cargo build --release)
43. ollama pull deepseek-ocr
```

## Phase 7 — Chat Panel (2 hours)
```
44. styles/chat.css
45. src/lib/markdown.js (marked renderer)
46. src/lib/events.js (event bus)
47. src/lib/prompts.js (prompt templates)
48. src/lib/process-notify.js
49. src/lib/alice-responses.js
50. src/ui/chat-panel.js (full chat UI + history sidebar + markdown)
51. Port chatSendMessage() from main.js
52. Wire process_message + process_turn Tauri commands
53. Replace old CHAT_SYSTEM with new Alice identity prompt
```

## Phase 8 — OCR + Search Panels (3 hours)
```
54. styles/ocr.css + src/ui/ocr-panel.js (upload, queue, preview, approve)
55. styles/search.css + src/ui/search-panel.js (progress, report, PDF list)
56. Wire Tauri commands for OCR + search
57. Add PDF opener integration (tauri-plugin-shell)
58. Add "Send to OCR" → "Approve to KB" pipeline
59. Test: full OCR + search + approve flows
```

## Phase 9 — Knowledge Base Panel (2 hours)
```
60. styles/kb.css + src/ui/kb-panel.js (graph canvas + 3 table tabs)
61. Canvas force-directed graph (Coulomb + spring, colored nodes, labeled edges)
62. Nodes table (searchable, filterable, deleteable)
63. Edges table (deleteable)
64. Files table (open/delete)
65. Wire to SurrealDB/Qdrant for live data
66. "Add node" / "Add edge" manual entry
67. Test: view graph, delete items, add manual entries
```

## Phase 10 — Sidebar + Polish (1.5 hours)
```
68. styles/sidebar.css + src/ui/sidebar.js (toggle + badges)
69. Keyboard shortcuts
70. Panel state persistence (localStorage: positions, visibility)
71. Final cleanup: remove old #hud, #chatbox, dead CSS
72. Slim main.js to bootstrapper (~100 lines)
```

## Phase 11 — TTS Toggle + Testing (1 hour)
```
73. Modify tts-player.js (gainNode + setMuted)
74. TTS toggle in status panel
75. Mouth animation: 3-branch render loop
76. End-to-end test: all 3 chat flows, all 5 panels, TTS toggle
77. Verify Rust compilation, frontend build
```

**Total: ~19 hours**

---

# Part 7 — Complete File Manifest

```
NEW FILES (49):
├── docker-compose.yml
├── searxng/
│   ├── settings.yml
│   └── limiter.toml
│
├── src-tauri/src/memory/
│   ├── mod.rs
│   ├── config.rs
│   ├── commands.rs
│   ├── working.rs
│   ├── conversation.rs
│   ├── episodic.rs
│   ├── semantic.rs
│   ├── graph.rs
│   ├── embeddings.rs
│   ├── extraction.rs
│   ├── summarizer.rs
│   ├── consolidator.rs
│   ├── retrieval.rs
│   ├── context.rs
│   ├── pipeline.rs
│   └── ingestion.rs
│
├── src-tauri/src/db/
│   ├── mod.rs
│   ├── surreal.rs
│   └── qdrant.rs
│
├── src-tauri/src/tools/
│   ├── mod.rs
│   ├── types.rs
│   ├── ocr.rs
│   └── meta_search.rs
│
├── src/ui/
│   ├── panel-system.js
│   ├── status-panel.js
│   ├── chat-panel.js
│   ├── ocr-panel.js
│   ├── search-panel.js
│   ├── kb-panel.js
│   └── sidebar.js
│
├── src/lib/
│   ├── ipc.js
│   ├── markdown.js
│   ├── drag-resize.js
│   ├── events.js
│   ├── prompts.js
│   ├── process-notify.js
│   └── alice-responses.js
│
├── styles/
│   ├── tokens.css
│   ├── panel.css
│   ├── status.css
│   ├── chat.css
│   ├── ocr.css
│   ├── search.css
│   ├── kb.css
│   └── sidebar.css

MODIFIED FILES (8):
├── index.html                              (strip inline styles, minimal DOM)
├── src/main.js                             (slim to bootstrapper, new CHAT_SYSTEM)
├── src/tts-player.js                       (add gainNode + setMuted)
├── src-tauri/Cargo.toml                    (add 9 crates)
├── src-tauri/src/lib.rs                    (mods, plugins, commands, state)
├── src-tauri/src/tts.rs                    (add stop_tts command)
├── src-tauri/capabilities/default.json     (add shell:allow-open)
├── package.json                            (add @tauri-apps/plugin-shell)
├── .gitignore                              (add data/)

DELETED FILES (1):
├── src-tauri/src/voice.rs                  (already removed — STT system)
```

---

# Part 8 — Error Handling Matrix

| Scenario | Handling |
|----------|----------|
| SurrealDB unavailable | 3x retry with backoff; fall back to working memory only |
| Qdrant unavailable | Skip vector search; use graph + semantic only |
| Ollama embedding fails | Skip embedding; store facts without vectors |
| LLM extraction fails | Skip extraction for this turn, log error |
| Graph >500 nodes in RAM | Partial load with LRU eviction to SurrealDB |
| Panel dragged off-screen | Clamp to viewport (min 20px visible) |
| Panel resize below minimum | Enforce minWidth/minHeight |
| OCR fails mid-processing | Show error in queue, [Retry] button, preserve partial |
| meta_search times out | Show partial results, warning banner, allow re-run |
| Session switch during streaming | Block switch, show tooltip |
| Delete active chat | Switch to newest remaining or create new |
| PDF deleted from disk after KB ingest | File tab shows "missing" badge, keep metadata |
| Tool binary not found | find_binary() checks 2 paths, clear error message |
| Malformed LLM JSON response | Regex extraction fallback; default to empty/neutral |
| Window resize while panels open | Proportional position adjustment, clamp to bounds |

---

# Part 9 — Startup Sequence

```bash
# 1. Pull required Ollama models (first time only)
ollama pull gemma4:e4b
ollama pull qwen3-embedding:0.6b
ollama pull deepseek-ocr

# 2. Start Docker services
docker compose up -d
# → surrealdb :8000, qdrant :6333/6334, searxng :8888, lightpanda :9222

# 3. Build external tool binaries
(cd /home/mka/Code/ocr_helper_system && cargo build --release)
(cd /home/mka/Code/meta_search_Js_render && cargo build --release)

# 4. Start Alice
npm run tauri dev

# App startup sequence:
# 1. Connect SurrealDB → create tables if missing
# 2. Connect Qdrant → create collections if missing
# 3. Load knowledge graph from SurrealDB into Petgraph (RAM)
# 4. Ping Ollama health
# 5. Mount status panel (top-left)
# 6. Mount sidebar (right-edge toggles)
# 7. Load PixiJS + Live2D model
# 8. Init TTS player
# 9. Restore panel positions from localStorage
# 10. Ready — status panel shows "Chat ready"
```
