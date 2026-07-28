# ALICE — Local AI Assistant with Knowledge Memory

A fully offline desktop AI assistant featuring an interactive **Live2D "Demon Girl" avatar** named Alice. Type messages, and the avatar responds with synthesized speech while animating facial expressions, lip sync, blinking, and head tracking. Alice remembers everything — conversations, documents, research — in a persistent knowledge graph.

Everything runs locally — no cloud services required. Built with Tauri 2, Rust, vanilla JavaScript, PixiJS, and ONNX models.

## Features

- **Streaming chat** — local LLM via Ollama (`gemma4:e4b`), streaming token-by-token with markdown rendering
- **Text-to-Speech** — Piper VITS voice synthesis with audio-timed lip sync, toggleable mute
- **Live2D avatar** — 7 expressions driven by LLM response tags, eye tracking, breathing, procedural blink
- **Multi-panel UI** — draggable, resizable, minimizable panels for chat, OCR, search, knowledge base
- **Persistent memory** — knowledge graph (Petgraph + SurrealDB), vector search (Qdrant), facts and episodes
- **Memory pipeline** — LLM entity/fact/relation extraction, summarization, importance scoring, embedding
- **Memory consolidation** — periodic deduplication, pruning, and reinforcement of stored knowledge
- **Document ingestion** — PDF OCR via `deepseek-ocr`, result preview, approve-to-knowledge-base flow
- **Literature search** — meta-search across Google Scholar, arXiv, PubMed, Semantic Scholar, and more
- **Service health dashboard** — live status of Ollama, SurrealDB, Qdrant, SearXNG, Lightpanda, TTS, embedding model
- **System doctor** — `doctor.sh` script for automated health checks across all services
- **Fully offline** — all models and services run locally in Docker containers

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                     Tauri 2 Desktop App                       │
│                                                              │
│  ┌─ JS Frontend ───────────────────────────────────────────┐ │
│  │  panel-system   chat-panel    ocr-panel   search-panel  │ │
│  │  kb-panel       status-panel  sidebar                   │ │
│  └──────────────────────┬──────────────────────────────────┘ │
│                         │ Tauri IPC (12 commands)             │
│  ┌─ Rust Backend ──────┼───────────────────────────────────┐ │
│  │  tts.rs             │                                   │ │
│  │  memory/  (mod, pipeline, graph, embeddings, extraction,│ │
│  │            summarizer, consolidator, retrieval, context,│ │
│  │            ingestion, working, conversation, episodic,  │ │
│  │            semantic)                                    │ │
│  │  db/      (surreal.rs, qdrant.rs)                      │ │
│  │  tools/   (ocr.rs, meta_search.rs)                     │ │
│  └──────────────────────┼───────────────────────────────────┘ │
└─────────────────────────┼────────────────────────────────────┘
                          │
    ┌─────────────────────┼─────────────────────┐
    │                     │                     │
┌───▼──────┐  ┌───────────▼──────┐  ┌──────────▼──────────┐
│ SurrealDB│  │     Qdrant       │  │  Ollama :11434      │
│  :8000   │  │   :6333/6334     │  │  gemma4:e4b (chat)  │
│ (struct) │  │   (vectors)      │  │  qwen3-embedding    │
└──────────┘  └──────────────────┘  │  deepseek-ocr (PDF) │
                                    └─────────────────────┘
    ┌─────────────────────┐
    │ SearXNG    :8888    │  ← meta-search (8 engines)
    │ Lightpanda :9222    │  ← JS page rendering
    └─────────────────────┘
```

### Tech Stack

| Layer | Technology |
|---|---|
| Desktop framework | Tauri 2 (Rust + WebView) |
| Frontend | Vanilla JS (ES modules), Vite 5 |
| 2D rendering | PixiJS 6 + pixi-live2d-display (Cubism 4) |
| LLM backend | Ollama (`gemma4:e4b`) |
| Embedding model | `qwen3-embedding:0.6b` (1024-dim) |
| Text-to-Speech | Sherpa-ONNX Piper VITS ("amy-low") |
| Structured memory | SurrealDB (conversations, facts, episodes, graph) |
| Vector memory | Qdrant (semantic search) |
| Knowledge graph | Petgraph (in-memory) + SurrealDB (persistence) |
| Literature search | SearXNG (8 engines) + Lightpanda (JS render) |
| Markdown rendering | marked |

## Prerequisites

- **Rust** (1.77+) — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **Node.js** (18+) — for Vite and npm packages
- **Docker** — for SurrealDB, Qdrant, SearXNG, Lightpanda containers
- **Ollama** — `ollama serve`
- **Tauri system dependencies**:
  - Arch: `sudo pacman -S webkit2gtk-4.1 libappindicator-gtk3`
  - Debian/Ubuntu: `sudo apt install libwebkit2gtk-4.1-dev libappindicator3-dev`

## Quick Start

```bash
# 1. Clone
git clone <repo-url> && cd Alice_AI_Demoness

# 2. Install dependencies
npm install

# 3. Pull Ollama models
ollama pull gemma4:e4b
ollama pull qwen3-embedding:0.6b
ollama pull deepseek-ocr       # optional — for PDF OCR

# 4. Download TTS model
mkdir -p tts/tts_piper_amy_low
curl -L -o /tmp/amy-low.tar.bz2 \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-amy-low.tar.bz2
curl -L -o /tmp/espeak-ng-data.tar.bz2 \
  https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/espeak-ng-data.tar.bz2
tar -xjf /tmp/amy-low.tar.bz2 -C tts/tts_piper_amy_low/ --strip-components=1
tar -xjf /tmp/espeak-ng-data.tar.bz2 -C tts/tts_piper_amy_low/

# 5. Start Docker services
docker compose up -d
# → SurrealDB :8000, Qdrant :6333/6334, SearXNG :8888, Lightpanda :9222

# 6. Verify system health
bash doctor.sh

# 7. Build external tools (optional — for OCR and meta-search)
(cd ../ocr_helper_system && cargo build --release)
(cd ../meta_search_Js_render && cargo build --release)

# 8. Run
npm run tauri dev
```

## Project Structure

```
├── index.html                     # Minimal DOM scaffolding (canvas + CSS links)
├── docker-compose.yml             # SurrealDB, Qdrant, SearXNG, Lightpanda containers
├── doctor.sh                      # Automated system health checker
├── vite.config.js                 # Vite config (Ollama API proxy)
├── package.json                   # JS dependencies
├── MASTER_PLAN.md                 # Full architecture & implementation plan
├── TEST_PLAN.md                   # Manual test matrix & performance benchmarks
│
├── src/                           # Frontend (vanilla JavaScript, ES modules)
│   ├── main.js                    # Bootstrap: PixiJS, Live2D model, animation loop
│   ├── tts-player.js              # Web Audio TTS queue + mute + mouth extraction
│   ├── lib/                       # Shared utilities (7 files)
│   │   ├── ipc.js                 # Tauri IPC + PDF opener
│   │   ├── markdown.js            # marked renderer (code copy, external links)
│   │   ├── drag-resize.js         # Panel drag + resize primitives
│   │   ├── events.js              # App-wide pub/sub event bus
│   │   ├── prompts.js             # Alice identity + 9 LLM prompt templates
│   │   ├── process-notify.js      # Background process status formatting
│   │   └── alice-responses.js     # Alice's auto-generated process messages
│   └── ui/                        # Panel modules (7 files)
│       ├── panel-system.js        # Draggable/resizable/minimizable panel framework
│       ├── status-panel.js        # 7-service health dashboard + log feed
│       ├── chat-panel.js          # Chat with markdown, session sidebar, streaming
│       ├── ocr-panel.js           # PDF upload → queue → preview → approve
│       ├── search-panel.js        # Meta-search progress → report → PDF list
│       ├── kb-panel.js            # Canvas graph viz + nodes/edges/files tables
│       └── sidebar.js             # Right-edge panel toggle buttons + shortcuts
│
├── styles/                        # CSS — design tokens + per-panel styles (9 files)
│   ├── tokens.css                 # 50+ CSS custom properties
│   ├── panel.css                  # Panel chrome, resize, minimize/maximize
│   ├── scrollbar.css              # Global scrollbar styling
│   ├── status.css                 # Service indicators, log feed
│   ├── chat.css                   # Bubbles, sidebar, markdown styling
│   ├── ocr.css                    # Upload zone, queue, preview
│   ├── search.css                 # Progress log, paper cards
│   ├── kb.css                     # Graph canvas, tables, tabs
│   └── sidebar.css                # Toggle button bar
│
├── src-tauri/                     # Rust backend
│   ├── Cargo.toml                 # 15 crate dependencies
│   ├── tauri.conf.json            # Tauri window & bundle config
│   └── src/
│       ├── main.rs                # Entry point (Wayland/X11 env vars)
│       ├── lib.rs                 # App setup, 12 Tauri commands, plugins
│       ├── tts.rs                 # Piper VITS TTS engine + commands
│       ├── memory/                # Memory subsystem (15 modules)
│       │   ├── mod.rs             # MemoryManager orchestrator + periodic consolidation
│       │   ├── config.rs          # MemoryConfig (URLs, model names)
│       │   ├── commands.rs        # 10 Tauri IPC commands
│       │   ├── working.rs         # Layer 1: Working memory (RAM, VecDeque)
│       │   ├── conversation.rs    # Layer 2: Conversation CRUD (SurrealDB)
│       │   ├── episodic.rs        # Layer 3: Episode store (SurrealDB + Qdrant)
│       │   ├── semantic.rs        # Layer 4: Facts/preferences (SurrealDB)
│       │   ├── graph.rs           # Layer 5: Petgraph knowledge graph
│       │   ├── embeddings.rs      # Ollama qwen3-embedding client
│       │   ├── extraction.rs      # LLM entity/fact/relation extraction
│       │   ├── summarizer.rs      # LLM turn summarization + importance scoring
│       │   ├── consolidator.rs    # Merge duplicates, prune, reinforce
│       │   ├── retrieval.rs       # Hybrid retrieval (graph + semantic + vector)
│       │   ├── context.rs         # Context builder: memory → prompt prefix
│       │   ├── pipeline.rs        # End-to-end: summarize → extract → store → embed
│       │   └── ingestion.rs       # Tool results → memory pipeline bridge
│       ├── db/                    # Database client wrappers (2 files)
│       │   ├── surreal.rs         # SurrealDB connect + CRUD
│       │   └── qdrant.rs          # Qdrant connect + upsert + search
│       └── tools/                 # External tool integration (4 files)
│           ├── mod.rs             # ToolRegistry + IngestTool trait + dispatch
│           ├── types.rs           # IngestionResult, Fact, Relation
│           ├── ocr.rs             # ocr-helper subprocess wrapper
│           └── meta_search.rs     # meta_search subprocess wrapper
│
├── searxng/                       # SearXNG config
│   ├── settings.yml               # 8 search engines configured
│   └── limiter.toml               # Rate-limiting for local network
│
├── public/DemonGirl/              # Live2D Cubism 4 model files
│   ├── demongirl.model3.json
│   ├── demongirl.moc3
│   ├── demongirl.physics3.json
│   └── *.exp3.json                # 7 expression files
│
├── models/                        # ONNX models (downloaded separately)
├── tts/tts_piper_amy_low/         # TTS voice model (~63 MB)
└── data/                          # Docker persistent volumes (gitignored)
```

## Services & Ports

| Port | Service | Purpose |
|------|---------|---------|
| 8000 | SurrealDB | Structured memory (facts, conversations, graph persistence) |
| 6333/6334 | Qdrant | Vector search (semantic/embedding memory) |
| 8888 | SearXNG | Multi-engine academic search (8 engines) |
| 9222 | Lightpanda | Headless JS page rendering (CDP) |
| 11434 | Ollama | LLM inference + embedding generation |

## Tauri IPC Commands

| Command | Direction | Purpose |
|---------|-----------|---------|
| `enrich_prompt` | JS → Rust | Inject memory context into system prompt before LLM call |
| `process_turn` | JS → Rust | Run memory pipeline after assistant response |
| `process_message` | JS → Rust | Combined: tool dispatch + prompt enrichment |
| `get_kb_data` | JS → Rust | Retrieve all knowledge graph nodes and edges |
| `kb_add_node` | JS → Rust | Add a node to the knowledge graph |
| `kb_add_edge` | JS → Rust | Add an edge between two graph nodes |
| `run_ocr` | JS → Rust | Execute OCR on a PDF file |
| `run_search` | JS → Rust | Execute meta-search on a topic |
| `ingest_text` | JS → Rust | Feed arbitrary text into the memory pipeline |
| `run_consolidation` | JS → Rust | Manually trigger memory consolidation |
| `generate_tts` | JS → Rust | Generate speech audio from text |
| `stop_tts` | JS → Rust | Stop in-progress TTS playback |

## Memory Layers

| Layer | Purpose | Storage | Lifetime |
|-------|---------|---------|----------|
| 1. Working | Current conversation context | RAM (VecDeque) | Session |
| 2. Conversation | Full message history | SurrealDB | Persistent |
| 3. Episodic | Important events | SurrealDB + Qdrant vectors | Persistent |
| 4. Semantic | Facts about user | SurrealDB | Persistent |
| 5. Knowledge Graph | Entity-relation graph | Petgraph (RAM) + SurrealDB sync | Persistent |
| 6. Vector | Semantic embeddings | Qdrant | Persistent |

### Memory Pipeline

```
User message → Assistant response
        │
        ▼
  process_turn()
        │
        ├── 1. Save to conversation store (SurrealDB)
        ├── 2. LLM summarization (gemma4:e4b)
        ├── 3. Entity extraction → graph nodes (Petgraph)
        ├── 4. Fact extraction → semantic store (SurrealDB)
        ├── 5. Relation extraction → graph edges (Petgraph)
        ├── 6. Importance scoring → episodic store if > 0.4
        ├── 7. Embedding → Qdrant vector store
        └── 8. Graph sync → SurrealDB
```

### Memory Consolidation (every 30 minutes)

- **Merge duplicates** — identical facts merged, counts summed, confidence averaged
- **Prune stale** — facts below confidence threshold with no recent updates removed
- **Reinforce** — frequently observed facts get confidence boost

## Ollama Models

| Model | Size | Purpose |
|-------|------|---------|
| `gemma4:e4b` | 9.6 GB | Chat, entity/fact/relation extraction, summarization |
| `qwen3-embedding:0.6b` | 639 MB | Embedding generation (1024-dim vectors) |
| `deepseek-ocr` | ~6.7 GB | PDF page OCR (optional) |

## Configuration

### Chat model

Set in `src/ui/chat-panel.js`, `chatSendMessage()`:
```js
body: JSON.stringify({ model: 'gemma4:e4b', stream: true, messages, ... })
```

### System prompt

Alice's identity and behavior rules are defined in `src/lib/prompts.js` as `CORE_IDENTITY`. Expression tags, extraction prompts, and all LLM templates are in that file.

### TTS speed

In `src-tauri/src/tts.rs`:
```rust
tts_config.speed = 1.10;  // 1.0 = normal, 1.10 = slightly faster
```

### Window size

In `src-tauri/tauri.conf.json`:
```json
"width": 1280, "height": 800
```

### Memory config

In `src-tauri/src/memory/config.rs` — all DB URLs, model names, embedding dimensions, and retrieval settings.

## Alice — Character Identity

Alice is a demoness bound to her master's service — serious, obedient, and precise. She does not roleplay or use cute language. Every response starts with an expression tag that drives the Live2D avatar:

| Tag | Expression | Use |
|-----|-----------|-----|
| `[neutral]` | (reset) | Default, factual answers, status reports |
| `[amazed]` | Amazed | Master's achievement, breakthrough |
| `[angry]` | Angry | Errors, failures, external disrespect |
| `[cry]` | Cry | Master distressed, bad news |
| `[love]` | Love | Genuine connection (rare) |
| `[nervous]` | Nervous | Uncertainty, risky operations |
| `[sleepy]` | Sleepy | Repetitive topics, exhausted |
| `[customise]` | Customise | Deep analysis, technical explanations |

## Panels

| Panel | Shortcut | Function |
|-------|----------|----------|
| System Status | (fixed top-left) | 7-service health dashboard, log feed, TTS/Blink/Look toggles |
| Chat | Ctrl+C | Markdown chat with session sidebar (new/delete/switch) |
| OCR | Ctrl+O | PDF upload → extraction → preview → approve to KB |
| Meta Search | Ctrl+S | Progress log → synthesis report → PDF list → approve |
| Knowledge Base | Ctrl+K | Canvas force graph + nodes/edges/files tables + CRUD |

All panels are draggable, resizable, minimizable, and persist position across restarts.

## Testing

```bash
# Quick smoke test (no Docker/Ollama needed)
bash doctor.sh --quick --no-docker --no-ollama

# Full system check (requires Docker + Ollama running)
bash doctor.sh

# Specific exclusions
bash doctor.sh --no-docker    # Skip Docker checks
bash doctor.sh --no-ollama    # Skip Ollama checks
```

The doctor script performs 40+ checks across 9 sections: environment, Docker services, Ollama models, Rust compilation, frontend build, integration points, database connectivity, code quality, and API tests. See `TEST_PLAN.md` for the full manual test matrix including UI interactions, end-to-end flows, and performance benchmarks.

## Development

```bash
npm run dev            # Vite dev server only (browser)
npm run tauri dev      # Full Tauri app in dev mode
npm run tauri build    # Production build → AppImage + .deb
```

## Roadmap

| Phase | Description | Status |
|---|---|---|
| 1 — Foundation | Tauri project, config, logging | Done |
| 2 — TTS | Piper TTS with sentence-level streaming | Done |
| 3 — Chat | Streaming Ollama with Live2D expressions | Done |
| 4 — UI | Multi-panel layout, markdown, session history | Done |
| 5 — Memory | SurrealDB + Qdrant + Petgraph knowledge graph | Done |
| 6 — Tools | OCR (PDF → text) + meta-search (literature) | Done |
| 7 — Pipeline | LLM extraction, summarization, consolidation | Done |

## License

MIT
