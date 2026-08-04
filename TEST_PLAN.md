# ALICE — System Test Plan

## Pre-Flight Checks

### 1. Docker Services

```bash
# Start all containers
docker compose up -d

# Verify all healthy
docker compose ps
# Expected: 4 services, all "Up" with healthy status

# Individual health checks
curl -s http://localhost:8000/health       # SurrealDB → 200
curl -s http://localhost:6333/health       # Qdrant → {"title":"qdrant - vector search engine","version":"..."}
curl -s http://localhost:8888/search?q=test&format=json  # SearXNG → JSON results
curl -s http://localhost:9222/json/version # Lightpanda → {"Browser":"...","Protocol-Version":"..."}
```

### 2. Ollama Models

```bash
ollama list
# Expected: gemma4:e4b, qwen3-embedding:0.6b present

# Quick embedding test
curl -s http://localhost:11434/api/embed -d '{"model":"qwen3-embedding:0.6b","input":["test"]}' | jq '.embeddings[0] | length'
# Expected: 1024

# Quick chat test
curl -s http://localhost:11434/api/chat -d '{"model":"gemma4:e4b","messages":[{"role":"user","content":"Say hi"}],"stream":false}' | jq '.message.content'
# Expected: non-empty string

# Automatic context selection
cd src-tauri
cargo test memory::config::tests::selects_context_buckets_and_respects_gpu_cap
# Expected: short/medium/large requests select 8K/16K/32K and never exceed the configured cap
```

### 3. Rust Compilation

```bash
cargo test --manifest-path crates/alice-core/Cargo.toml
cargo test --manifest-path crates/alice-sidecar/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
# Expected: all non-live tests pass and both desktop adapters compile
```

### 4. Sidecar Protocol

```bash
npm run sidecar:build
npm run sidecar:diagnostic
# Expected: 29 methods advertised; reads, Quick Extract, offline ingestion,
# cancellation, structured errors, allowlist, and clean shutdown report "ok"

npm run sidecar:diagnostic:live
# Expected with Docker/Ollama running: dbReady=true and real graph, file,
# chat-session, and search-history counts are returned without mutations.

npm run sidecar:diagnostic:mutations
# Expected with Docker running: one unique chat session is written, its two
# messages are read back, the session is deleted, and recordRemaining=false.

npm run sidecar:diagnostic:features
# Expected with Docker/Ollama running: valid WAV, streamed chat, chat/search
# cancellation, reversible KB ingestion, no new search history, and clean
# shutdown all report ok.

npm run sidecar:diagnostic:ocr
# Expected with deepseek-ocr installed: non-empty text, a correlated ocr-started
# event, temporaryOutputsRemaining=0, and cleanShutdown=true. Set
# ALICE_OCR_FIXTURE to select a specific local PDF.
```

### 5. Frontend and Electron Build

```bash
npm run check
npm run electron:pack
# Expected: frontend tests/build pass and release/electron/linux-unpacked
# contains executable resources/bin/alice-sidecar, tools/ocr-helper,
# tools/meta_search, the Piper model, and app.asar. electron:inspect reports ok.
```

`npm run test` also enforces the Phase 4 UI boundary: feature modules may not
access Tauri/Electron globals, raw IPC implementations, or the Ollama proxy
directly, and OCR/research panels may not contain simulated success paths.

---

## Unit Tests — Rust Backend

### DB Layer

| Test | Command / Code | Expected |
|------|---------------|----------|
| SurrealDB connect | `SurrealDB::connect("ws://localhost:8000", "alice", "memory", "root", "root")` | Ok(SurrealDB) |
| SurrealDB ensure_table | `db.ensure_table("test_table")` | Ok(()) |
| SurrealDB insert | `db.insert("test_table", json!({"key":"val"}))` | Ok(()) |
| SurrealDB query | `db.query("SELECT * FROM test_table")` | Ok(vec with 1 row) |
| Qdrant connect | `QdrantDB::connect("http://localhost:6334")` | Ok(QdrantDB) |
| Qdrant ensure_collection | `qdrant.ensure_collection("test", 1024)` | Ok(()) |
| Qdrant upsert | `qdrant.upsert("test", 1, vec![0.1; 1024], "hello")` | Ok(()) |
| Qdrant search | `qdrant.search("test", vec![0.1; 1024], 5)` | Ok(vec with 1 result) |

### Memory Layer

| Test | Code | Expected |
|------|------|----------|
| WorkingMemory add | `wm.add_message("user", "hello")` → `wm.get_context_window().len()` | 1 |
| WorkingMemory overflow | Add 60 msgs | max 50 in queue |
| Conversation store turn | `store.save_turn("s1", "user", "hi")` | Ok(()) |
| Conversation get session | `store.get_session("s1")` | Ok(vec with 1 record) |
| Semantic upsert | `sem.upsert_fact("master", "prefers", "Rust", 0.9)` | Ok(()) |
| Semantic duplicate upsert | Call same upsert again | count=2, confidence updated |
| Semantic search | `sem.search_facts("Rust")` | Ok(vec with 1+ fact) |
| Graph upsert node | `graph.upsert_node("Rust", Concept)` | Returns NodeIndex |
| Graph upsert duplicate | Call same name again | Same NodeIndex, updated time |
| Graph upsert edge | `graph.upsert_edge(fi, ti, "PREFERS", 0.9)` | Edge created |
| Graph neighbors | `graph.neighbors("Rust", 2)` | Returns entity names |
| Embeddings | `embedder.embed("hello world").await` | Ok(vec.len() == 1024) |

### Extraction Pipeline

| Test | Code | Expected |
|------|------|----------|
| Typed extraction | `extractor.extract_all("I use Rust and Tauri")` | Typed Rust/Tauri entities and a grounded relation |
| Document extraction | `extractor.extract_document(text, filename)` | All chunks processed and results merged/deduplicated |
| Relation endpoints | Parse a relation whose endpoints are absent from `entities` | Endpoints are added as graph entities |
| Summarizer | `summarizer.summarize_turn("hello","hi there")` | Ok(non-empty string) |
| Importance scoring | `summarizer.score_importance("remember I like Rust","noted")` | Ok(> 0.3) |

### Pipeline

| Test | Code | Expected |
|------|------|----------|
| process_turn | `pipeline.process_turn("I like Rust", "Noted, master")` | Ok(()) — conversation, facts, graph all populated |
| ingestion | `ingestion.ingest(ingestion_result)` | Ok("Ingested: N nodes...") |
| Retrieval | `retrieval.retrieve("Rust programming")` | Ok(RetrievedContext with facts/entities) |
| Context builder | `ContextBuilder::build_memory_context(&ctx)` | String with memory sections |
| Enrich context | `manager.enrich_context("Rust", base_prompt)` | String with memory prefix |

### Consolidation

| Test | Code | Expected |
|------|------|----------|
| Merge duplicates | Add 2 identical facts → run consolidator | merges >= 1 |
| Prune low confidence | Add fact with confidence 0.05, old timestamp → run | pruned >= 1 |
| Reinforce | Add fact with count=5, confidence=0.7 → run | reinforced >= 1 |
| Full run | `consolidator.run().await` | Ok(report with counts) |
| Tauri command | `invoke('run_consolidation')` | JSON with merges/pruned/reinforced |

### Tool Integration

| Test | Code | Expected |
|------|------|----------|
| ToolRegistry dispatch prompt | `ToolRegistry::dispatch_prompt("search papers")` | String with tool instructions |
| OcrTool availability | `ocr_tool.is_available()` | true/false |
| MetaSearchTool availability | `search_tool.is_available()` | true/false |
| Tool execute (stub) | `registry.execute("ocr", "test.pdf", Some("test.pdf"))` | Ok or error with reason |

---

## Integration Tests — Rust Backend

### MemoryManager Lifecycle

```
Test: Full MemoryManager Init
1. Create MemoryConfig::default()
2. Call MemoryManager::new(config)
3. Verify: Ok(manager)

Test: Conversation Flow
1. manager.process_with_tools("hello", base_prompt) → enriched prompt
2. Verify: returned prompt != base_prompt (if memory has context)
3. manager.process_turn("hello", "hi master")
4. Verify: Ok(())

Test: Read-only KB Graph and File Cascade Delete
1. invoke get_kb_data → { nodes: [], edges: [], files: [] }
2. ingest a document and verify graph nodes appear with its filename as source
3. verify Graph, Nodes, and Edges tabs expose no add/delete controls
4. invoke delete_file_record { name, deleteNodes: true }
5. verify its file record, document vectors, and source-tagged graph nodes are removed

Test: Ingest Text
1. invoke ingest_text { text: "master uses Python", source: "test", sourceType: "manual" }
2. Verify: returns "Ingested: N nodes..."
3. invoke get_kb_data → graph expanded

Test: Tool Dispatch (requires Ollama)
1. Send message: "search papers on Rust async"
2. Verify: LLM dispatch → tool_status in response
3. If search binary available → runs and returns results
4. If not → graceful error
```

---

## Frontend Tests — JavaScript

### Panel System

| Test | How | Expected |
|------|-----|----------|
| Create panel | `createPanel('test', 'Test', {})` in console | Panel appears on screen |
| Drag panel | Click-drag header | Panel follows mouse, clamped to viewport |
| Resize panel | Drag bottom-right corner | Panel resizes, enforces min/max |
| Minimize toggle | Click `—` button | Collapses to titlebar only (32px height) |
| Maxmimize toggle | Click `□` button | Fills viewport, second click restores |
| Close panel | Click `×` button | Panel hides |
| Re-open panel | Call `createPanel('test', ...)` again | Panel shows at last position |
| Focus/Z-order | Click panel | z-index increments, panel moves to front |
| Save/restore state | Refresh page | Panels restored to last positions |

### Status Panel

| Test | How | Expected |
|------|-----|----------|
| Service health dots | Wait 5s after page load | Green/yellow/red dots for each service |
| Ollama dot | Must be running | Green with model count |
| SurrealDB dot | Docker must be up | Green with URL |
| Qdrant dot | Docker must be up | Green with URL |
| TTS dot | Always | Blue/green |
| Embedding dot | Always | Blue/green |
| Log feed | Type a message in chat | Log entry appears |
| TTS toggle | Click TTS button | Icon changes, log shows "TTS enabled/muted" |

### Chat Panel

| Test | How | Expected |
|------|-----|----------|
| Send message | Type "hello" and press Enter | Message appears as user bubble |
| Streaming response | Wait for Ollama | Assistant text streams in, markdown rendered |
| Expression tag | Response starts with `[tag]` | Live2D face changes appropriately |
| Markdown rendering | Response with `**bold**`, `code`, lists | Rendered as formatted HTML |
| Code block copy | Click "Copy" on code block | Code copied to clipboard |
| New session | Click "+ New" in sidebar | Empty chat, session list shows new item |
| Switch session | Click another session | Messages change to that session |
| Delete session | Select session, click "Delete session" | Session removed from list |
| Auto-title | First message sent | Session title updates to first 40 chars |
| System messages | Tool completes (OCR/search) | Centered gray message appears |

### OCR Panel

| Test | How | Expected |
|------|-----|----------|
| Open panel | Click 📄 in sidebar or Ctrl+O | OCR panel appears |
| Upload zone | Drag a .pdf onto zone | File queued |
| Invalid file | Drag a .txt file | Error notification shown |
| Browse upload | Click upload zone → select PDF | File queued |
| Processing | After upload, wait | Progress bar shown (Tauri mode) |
| Complete | After processing | [View] and [Approve] buttons appear |
| View result | Click [View] | Markdown preview with ## Page headings |
| Back to list | Click "← Back" | Returns to queue list |
| Approve | Click [Approve] | KB ingestion triggered, status changes |
| Discard | Click [Discard] | Item removed from queue |
| Failed retry | If OCR fails | [Retry] button appears |

### Meta Search Panel

| Test | How | Expected |
|------|-----|----------|
| Open panel | Click 🔍 in sidebar or Ctrl+S | Search panel appears |
| Gathering mode | Select Knowledge document / Scientific papers | Description and topic placeholder update |
| Search history | Restart app after a completed search | Topic, mode, PDF count, and processing state load from SurrealDB |
| View report | Click ▶ on history item | KB-ready document, downloaded PDFs, and analyzed sources render |
| Progress display | Start a search | Log entries appear with progress bar |
| Parse noisy output | Run search with Lightpanda CDP warnings | Final JSON report loads instead of “Parse error” |
| PDF open | Click Open on a downloaded document | PDF opens in system viewer |
| Copy citation | Click 📋 on analyzed source | Citation copied, button says "Copied!" |
| Knowledge document | Click Add knowledge document to KB | Markdown is saved as a KB file and ingested |
| Quick Extract | Click Quick on a PDF | Text and method/status persist in `search_document` |
| PDF OCR | Click OCR on a PDF | OCR text and method/status persist in `search_document` |
| Batch integration | Click Quick Extract all → KB / OCR all PDFs → KB | Documents process sequentially, persist, and enter KB |
| Processed history | Restart app and open the topic | Processed/In KB states remain and View text lazy-loads output |

### Knowledge Base Panel

| Test | How | Expected |
|------|-----|----------|
| Open panel | Click 🧠 in sidebar or Ctrl+K | KB panel appears |
| Graph tab | Click "Graph" | Canvas with force-directed nodes/edges |
| Node click | Click a node on canvas | Detail panel shows name, type, relations |
| Node drag | Drag a node | Node repositions, others stay |
| Canvas zoom | Scroll wheel on canvas | Zoom in/out |
| Type filter | Select filter dropdown → "Tool" | Only Tool nodes shown |
| Nodes tab | Click "Nodes" | Read-only table of all nodes |
| Search nodes | Type in search box | Table filters |
| Edges tab | Click "Edges" | Read-only table of all relations |
| Files tab | Click "Files" | Table of ingested files |
| Graph controls | Inspect Graph, Nodes, and Edges tabs | No manual add/delete controls |
| Delete document | Delete a row in Files and confirm | File, vectors, and its source-tagged graph nodes are removed |

### Sidebar

| Test | How | Expected |
|------|-----|----------|
| Toggle panels | Click sidebar buttons | Panels show/hide |
| Active state | Panel visible | Sidebar button gets blue border |
| Keyboard shortcuts | Ctrl+C, Ctrl+O, Ctrl+S, Ctrl+K | Respective panels toggle |

### TTS Toggle

| Test | How | Expected |
|------|-----|----------|
| TTS ON (default) | Send message, wait for response | Audio plays, mouth animates |
| TTS OFF | Click 🔊 → 🔇 in status panel | Audio stops if playing, future messages silent |
| Mouth with TTS OFF | Watch avatar while TTS off | Mouth stays idle (gentle breathing) |
| Mouth with TTS ON | Watch avatar while TTS speaks | Mouth moves in sync with audio |

### Live2D

| Test | How | Expected |
|------|-----|----------|
| Model loaded | Page loads | DemonGirl visible on canvas |
| Expressions | LLM response with [love], [angry], etc. | Face changes to match |
| Breathing | Always | Subtle ParamBreath oscillation |
| Blink | Periodic | Eyes close briefly every 2-5 seconds |
| Mouse look | Move mouse | Eyes and head follow cursor |
| Mouse look OFF | Toggle in status panel | Eyes/head stay centered |
| Zoom | Scroll wheel on background | Model scales |
| Pan | Drag on background | Model repositions |

---

## End-to-End Flows

### Flow 1: Normal Chat

```
1. Start app: npm run tauri dev
2. Status panel shows green dots for available services
3. Type "Hello Alice" in chat, press Enter
4. Wait for streaming response
5. Verify: Response has [expression_tag], markdown renders, TTS speaks, mouth moves
6. Verify: Status log shows chat activity
7. Switch TTS OFF → send another message → silent, mouth idle
```

### Flow 2: Chat with Memory

```
1. Send: "Remember that I use Rust for all my projects"
2. Wait for response
3. Send: "What programming language do I use?"
4. Verify: Alice recalls "Rust" from memory context (once enrichment is wired with DB)
5. Send: "Remember I also like Python for scripting"
6. Wait for response
7. Send: "What languages do I use?"
8. Verify: Alice mentions both Rust and Python
```

### Flow 3: Multisession Chat

```
1. Chat normally for 2-3 messages
2. Click "+ New" in chat sidebar → new session
3. Send a different topic
4. Switch back to first session → messages preserved
5. Delete second session → confirm dialog → removed
6. Verify: Only first session remains
```

### Flow 4: KB Visualization

```
1. Open KB panel (Ctrl+K)
2. Ingest a document that mentions "Rust" and "master"
3. Approve the extracted document
4. Refresh the KB after extraction completes
5. Verify: Both nodes and edge visible in Graph tab
6. Click "Rust" node → detail panel shows info
7. Switch to Nodes tab → both nodes listed
8. Switch to Edges tab → edge listed
9. Ingest a second document and verify its cluster is spaced apart from the first
10. Click "Re-layout" and verify clusters remain separated and the full graph fits
11. Click "AI Analyze & Improve", confirm, and wait for the result
12. Verify the status shows batch progress and an ETA; cloud models may show `2× cloud`
13. Interrupt a long run after at least one batch, retry, and verify completed batches are reported as resumed
14. Verify high-confidence aliases collapse into one node and all incoming/outgoing relations point to it
15. Verify unclear node names gain concise canonical names and meanings without changing document nodes
16. Verify vague relation labels become concise semantic labels without changing direction or facts
17. Run analysis again and verify nodes with existing meanings are skipped by the incremental plan
18. Click a node and verify direct relations animate as bright wavy lines and connected nodes use a different glow color
19. Verify its meaning appears without a long relation-description list
20. Switch to Files and delete one source document
21. Verify shared merged nodes remain when the second document still owns them
22. Delete the final source and verify its vectors, nodes, and relations are removed
```

### Flow 5: OCR Pipeline (requires deepseek-ocr)

```
1. Open OCR panel (Ctrl+O)
2. Upload a test PDF
3. Wait for processing to complete
4. Click [View] → verify markdown preview
5. Click [Approve] → KB ingestion
6. Open KB panel → verify new nodes/edges added
```

### Flow 6: Meta Search (requires external tools)

```
1. Open Search panel (Ctrl+S)
2. Select "Knowledge document", enter "attention mechanisms", and click Gather
3. Verify: progress completes without a JSON parse error
4. Verify: the KB-ready document and downloaded-document inventory appear
5. Click Add knowledge document to KB
6. Run another topic in "Scientific papers" mode
7. Click Quick Extract all → KB (then test OCR all PDFs → KB on a small set)
8. Restart the app and verify both topics, documents, processed text, and KB states persist
9. Check KB Files and Graph tabs for the ingested knowledge document and papers
```

### Flow 7: Panel Persistence

```
1. Arrange panels to custom positions
2. Minimize one, maximize another
3. Refresh the page (F5)
4. Verify: All panels restore to saved positions
5. Verify: Minimized/maximized state preserved
```

### Flow 8: Service Failure Gracefulness

```
1. Stop SurrealDB: docker compose stop surrealdb
2. Refresh app
3. Verify: Status panel shows red dot for SurrealDB
4. Verify: Chat still works (falls back to working memory)
5. Start SurrealDB: docker compose start surrealdb
6. Verify: Dot turns green after health check
```

---

## Performance Benchmarks

| Metric | Target | How to Test |
|--------|--------|-------------|
| Cold start time | < 5s | Time from `npm run tauri dev` to "Ready" log |
| First chat response | < 3s streaming start | Time from Enter to first token |
| Embedding latency | < 500ms | Time embed("test") in Rust |
| LLM extraction | Hardware-dependent | Log entities/facts/relations and disconnected components |
| LLM graph cleanup | Hardware-dependent | Run ignored `live_cleanup_analyzer_finds_aliases` test |
| Ollama context | 8K/16K/32K, ≤ configured cap | Inspect `ollama ps` during short and dense requests |
| Consolidation sweep | < 5s for 1000 facts | Run consolidate with many facts |
| Panel drag FPS | 60 FPS | Drag panel quickly — no jank |
| Fullscreen render | Near 60 FPS in Auto | Maximize window and inspect System Status render telemetry |
| Resize responsiveness | No multi-frame stalls | Continuously resize window; verify temporary 30 FPS/low-resolution mode |
| Markdown render | < 100ms for 10KB | Render large markdown block |

---

## Quick Smoke Test (Run After Every Change)

```bash
# 1. Compile
cd src-tauri && cargo check 2>&1 | grep -c "error"  # Expected: 0

# 2. Build frontend
cd .. && npm run build 2>&1 | tail -1  # Expected: "✓ built in Xs"

# 3. Docker health
docker compose ps --format json | grep -c "healthy"  # Expected: 4

# 4. Ollama health
curl -s http://localhost:11434/api/tags | jq '.models | length'  # Expected: >= 2
```
