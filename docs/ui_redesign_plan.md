# UI Redesign — Multi-Panel Layout System

## 1. Current UI Analysis & Problems

### 1.1 — Structural Issues

| Issue | Location | Severity |
|-------|----------|----------|
| Single monolithic `main.js` (655 lines) | `src/main.js` | High — unmaintainable, no separation of concerns |
| All CSS inline in `<style>` tag (114 lines) | `index.html:14-128` | High — no design tokens, impossible to theme |
| HUD mixes state, controls, status, meters | `index.html:132-157` | Medium — status panel is just a single string |
| Chat messages render as `textContent` only | `main.js:490-492` | High — `marked` exists in deps but unused for messages |
| No markdown body rendering | `main.js:492` | High — `textContent` strips all rich formatting |
| Backdrop blur on chatbox | `index.html:54` | Medium — user explicitly wants no blur (solid background) |
| Dead code: lip-sync references removed voice feature | `main.js:239-242` | Low — misleading to developers |
| `chat-hidden` toggle logic inverted at load | `main.js:648-649` | Low — confusing boot state |

### 1.2 — Missing Features (User Requirements)

| Requirement | Status |
|-------------|--------|
| **Draggable panels** | Not implemented — all panels are fixed `position: fixed` |
| **Resizable panels** | Not implemented |
| **Minimizable status panel** | Not implemented — HUD is always visible, no collapse |
| **Markdown rendering in chat** | Not implemented — messages use `textContent` |
| **Chat history management** | Not implemented — single ephemeral `chatHistory[]` array |
| **Multi-chat sessions (new, delete, continue)** | Not implemented |
| **OCR panel with PDF upload** | Not implemented |
| **OCR result preview & approve flow** | Not implemented |
| **Meta search panel with progress** | Not implemented |
| **Meta search report & PDF list** | Not implemented |
| **PDF opener integration** | Not implemented — Tauri has `shell.open()` capability |
| **Knowledge base visualization** | Not implemented |
| **KB node/edge/file management & deletion** | Not implemented |
| **Service health dashboard** | Not implemented |

### 1.3 — CSS/DOM Inconsistencies

- No CSS custom properties (variables) — every color/value is a magic number
- `z-index` values are arbitrary (10, 100, 101) with no stacking context strategy
- `transition` duplicates across rules instead of using a shared variable
- Scrollbar customization only applied to `#chat-messages`, not reusable
- No responsive breakpoints — fixed widths that break on small screens

---

## 2. Architecture Overview

```
src/
├── main.js                    # App bootstrap + Live2D + animation loop (slimmed)
│
├── ui/                        # NEW — UI modules
│   ├── panel-system.js        # Draggable/resizable/minimizable panel framework
│   ├── status-panel.js        # Service health dashboard + status logger
│   ├── chat-panel.js          # Chat UI + markdown rendering + history
│   ├── ocr-panel.js           # PDF upload + OCR preview + KB approval
│   ├── search-panel.js        # Meta-search progress + report + PDF list
│   ├── kb-panel.js            # Knowledge graph visualization + management
│   └── sidebar.js             # Panel toggle sidebar (shows which panels are open)
│
├── lib/                       # NEW — shared utilities
│   ├── ipc.js                 # Tauri IPC abstraction (window.__TAURI_INTERNALS__)
│   ├── markdown.js            # marked renderer with custom styling
│   └── drag-resize.js         # Drag + resize primitive (used by panel-system)
│
├── tts-player.js              # (unchanged)
│
styles/
├── tokens.css                 # CSS custom properties (colors, spacing, z-index)
├── panel.css                  # Panel chrome (header, body, resize handles)
├── status.css                 # Status panel styles
├── chat.css                   # Chat panel + message bubble styles
├── ocr.css                    # OCR panel styles
├── search.css                 # Search panel styles
├── kb.css                     # Knowledge base panel styles
└── sidebar.css                # Toggle sidebar styles

index.html                     # Compiled CSS includes + minimal DOM scaffolding
```

### 2.1 — Z-Index Stacking Strategy

```
Layer        z-index    Used for
─────        ───────    ────────
Canvas       0          PixiJS canvas (background)
Panels       100-199    Draggable panels (auto-increment on drag)
Panel header 200        Active drag handle overlay
Sidebar      300        Panel toggle sidebar (always on top of panels)
Tooltip       1000       Hover tooltips
Modal         2000       Confirmation dialogs
```

### 2.2 — Panel Lifecycle

```
User clicks sidebar toggle
        │
        ▼
Panel created (if first time) or panel restored (if previously hidden)
        │
        ▼
Panel rendered at last-known position (center of screen if new)
        │
        ├── User drags header → panel follows mouse
        ├── User drags resize corner → panel resizes (min 300×200, max window-100)
        ├── User clicks minimize → panel collapses to titlebar only
        ├── User clicks maximize → panel fills viewport (below status panel)
        └── User clicks close → panel hides, state preserved
        │
        ▼
When system triggers panel update (OCR complete, search done, etc.):
  - If panel is visible → update content in-place
  - If panel is minimized → flash/badge the sidebar toggle
  - If panel is closed → auto-show or badge toggle
```

---

## 3. Panel Designs (Detailed)

### 3.1 — Status Panel (Fixed Top-Left, Minimizable)

```
┌────────────────────────────────────────────────────┐
│ ▼ SYSTEM STATUS                          [_] [×]   │  ← minimize only (× disabled)
├────────────────────────────────────────────────────┤
│ 🔵 Ollama      ████████████  gemma4:e4b   9.6 GB  │
│ 🟢 SurrealDB   ████████████  ws://localhost:8000   │
│ 🟢 Qdrant      ████████████  :6334 (1024-dim)     │
│ 🟡 SearXNG     ████████████  8 engines, :8888      │
│ ⚪ Lightpanda  ░░░░░░░░░░░░  CDP :9222 (offline)   │
│ 🟢 TTS         ████████████  Piper amy-low, :16000 │
│ 🟢 Embedding   ████████████  qwen3-embedding:0.6b  │
│ ─────────────────────────────────────────────────  │
│ 📋 14:32:01  Chat ready                            │
│ 📋 14:32:05  User: "search papers on attention"    │
│ 📋 14:32:06  ▶ meta_search started (10 papers)      │
│ 📋 14:33:45  ✓ meta_search done (8 papers analyzed) │
│ 📋 14:34:00  ▶ OCR: paper.pdf page 1/12            │
│ ─────────────────────────────────────────────────  │
│ Memory: 142 nodes · 89 edges · 34 facts · 12 epi.  │
└────────────────────────────────────────────────────┘

MINIMIZED STATE:
┌─────────────────────┐
│ ▶ SYSTEM STATUS [▼] │  ← single-line, click to expand
└─────────────────────┘
```

**Details:**
- **Fixed position**: `top: 8px; left: 8px` — never moves, never closes
- **Width**: `340px` default, `100% - 16px` on narrow screens
- **Max height**: `60vh`, scrollable overflow in log section
- **Service indicators**: Each row shows service name, colored dot (green/red/yellow/gray for offline), status text, and a thin bar for health
- **Log feed**: Last 50 status messages, newest at bottom, auto-scroll; older messages dim to 40% opacity
- **Memory stats**: Footer row showing persistent counts from SurrealDB/Qdrant queries
- **Minimize**: Toggle between expanded and single-line collapsed state (40px height)
- **No close**: Status panel is permanent — close button is visually disabled

**Color coding for service dots:**
```
🟢 Green  — Connected, healthy, responded within timeout
🟡 Yellow — Connected but slow (>1s response) or degraded
🔵 Blue   — Connected but model not loaded (Ollama models)
🔴 Red    — Connection failed after retries
⚪ Gray   — Service not configured / intentionally offline
```

### 3.2 — Chat Panel (Movable, No Blur, Markdown, History)

```
┌──────────────────────────────────────────────────────────┐
│ ≡ Chat Sessions                    [_] [□] [×]           │  ← drag handle
├────────────────┬─────────────────────────────────────────┤
│ ┌────────────┐ │  ┌─────────────────────────────────┐    │
│ │ Sidebar    │ │  │                                 │    │
│ │            │ │  │  [love] Hello master! How can   │    │
│ │ ◎ New Chat │ │  │  I help you today?             │    │
│ │            │ │  │                                 │    │
│ │ ────────── │ │  │         I need help with Rust   │    │
│ │            │ │  │         async programming. →│    │    │
│ │ ● Rust Q   │ │  │                                 │    │
│ │   yesterday│ │  │  [neutral] Sure! Rust async     │    │
│ │   12 msgs  │ │  │  uses futures and tokio...      │    │
│ │            │ │  │  ```rust                        │    │
│ │ ○ Papers   │ │  │  async fn foo() { ... }         │    │
│ │   Jul 25   │ │  │  ```                            │    │
│ │    5 msgs  │ │  │                                 │    │
│ │            │ │  │  ─────────────────────────────  │    │
│ │ ○ Setup    │ │  │                                 │    │
│ │   Jul 24   │ │  │  ┌──────────────────────────┐   │    │
│ │    8 msgs  │ │  │  │ Type a message...      ↑ │   │    │
│ │            │ │  │  └──────────────────────────┘   │    │
│ │ ────────── │ │  └─────────────────────────────────┘    │
│ │ 🗑 Delete  │ │                                         │
│ └────────────┘ │                                         │
└────────────────┴─────────────────────────────────────────┘
```

**Details:**

**A. Chat History Sidebar (left, resizable split)**
- **New Chat button** (◎) — creates fresh `chatHistory[]` session, stores old session
- **Session list** — each item shows: indicator (● = active, ○ = inactive), auto-generated title from first user message (truncated 25 chars), date, message count
- **Click session** — switches chat view, loads messages from SurrealDB (or from local `sessions` Map if not yet persisted)
- **Delete button** (🗑) — shows at bottom of sidebar or per-session on hover; confirmation dialog; deletes from SurrealDB + local state
- **Sidebar toggle** — collapse/expand sidebar with thin handle at the split edge
- **Empty state**: "No saved chats. Start a conversation to create one."

**B. Message Bubbles (right, scrollable)**
- **Solid background** — `background: rgba(22,22,32,0.92)` (no blur, nearly opaque)
- **Markdown rendering** — all messages rendered via `marked.parse()` with `.innerHTML`, not `.textContent`
- **Code blocks** — syntax-highlighted via a lightweight highlighter (or just `pre code` styling as currently, which is already adequate)
- **User messages**: right-aligned, blue-tinted bubble
- **Assistant messages**: left-aligned, dark bubble
- **System/tool messages**: centered, gray italic, smaller font (e.g., "Searching for papers...")
- **Timestamp hover**: on hover over any message, show timestamp tooltip

**C. Input Area (bottom)**
- **Textarea**: auto-expanding, `Enter` to send, `Shift+Enter` for newline
- **Send button**: arrow icon, disabled during streaming
- **Character count**: optional, shown when message > 500 chars

**D. Panel Chrome**
- **Drag handle**: whole header bar
- **Resize**: bottom-right corner drag handle (8px grab area, visual indicator on hover)
- **Minimize**: collapse to titlebar only (40px)
- **Maximize**: fill viewport
- **Close**: hide panel, preserve session state
- **Default position**: bottom-center (`left: 25%; bottom: 24px; width: 50%; min-width: 480px`)
- **Default size**: 600×400px
- **Constraints**: min 400×300px, max 95vw × 85vh

### 3.3 — OCR Panel (PDF Upload → Preview → Approve)

```
┌──────────────────────────────────────────────────────────┐
│ ≡ OCR Document Processor                [_] [□] [×]      │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ┌──────────────────────────────────────────────────┐   │
│  │                                                  │   │
│  │         📄  Drop PDF here or click to browse     │   │
│  │              (max 100 MB per file)               │   │
│  │                                                  │   │
│  └──────────────────────────────────────────────────┘   │
│                                                          │
│  ── History ─────────────────────────────────────────   │
│                                                          │
│  ┌─────────────────────────────────────────────────┐    │
│  │ ✓ paper.pdf              12 pages · 15,432 chars│    │
│  │   Processed 2 min ago       [View] [Approve ✗]  │    │
│  ├─────────────────────────────────────────────────┤    │
│  │ ● thesis.pdf  (page 3/45)    4,210 chars so far │    │
│  │   Processing...  ████████░░░░ 15%               │    │
│  └─────────────────────────────────────────────────┘    │
│                                                          │
└──────────────────────────────────────────────────────────┘

─── After clicking [View] on a completed item ───

┌──────────────────────────────────────────────────────────┐
│ ≡ OCR: paper.pdf                         [_] [□] [×]    │
│  ← Back to list                                         │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ## Page 1                                               │
│  This is the extracted text from page 1 of the PDF...    │
│                                                          │
│  ## Page 2                                               │
│  More extracted content with **markdown formatting**...  │
│                                                          │
│  [code blocks, tables, lists rendered via marked]        │
│                                                          │
│  ──────────────────────────────────────────────────────  │
│                                                          │
│  [✓ Approve & Add to Knowledge Base]  [✗ Discard]       │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

**Details:**

**A. Upload Zone**
- **Drag-and-drop** — full drop zone highlights on dragover
- **Click-to-browse** — opens native file picker (`.pdf` filter)
- **Multiple files** — queue multiple PDFs for sequential processing
- **Validation**: file type check (`.pdf` extension + `%PDF` magic bytes), size cap (100 MB)
- **Visual feedback**: drop zone pulses green on valid file hover, red on invalid

**B. Processing Queue (History List)**
- Each item shows: filename, page count/total, extracted char count, timestamp
- **Status indicators**:
  - `● Processing...` with progress bar (pages done / total)
  - `✓ Complete` with [View] and [Approve] buttons
  - `✗ Failed` with error message and [Retry] button
- **View button** → switches to markdown preview mode for that document
- **Approve button** → sends extracted text to memory ingestion pipeline, creates KB node for the document, stores in SurrealDB. Button changes to ✓ Approved (green, disabled).

**C. Markdown Preview (single document view)**
- **Back button** (←) to return to list
- Full extracted text rendered via `marked`
- Page headings as `## Page N`
- Scrollable body
- **Approve & Add to KB** button at bottom — same as history list approve
- **Discard** button — removes from local queue without persisting

**D. Panel Chrome**
- Default size: 520×500px
- Default position: center-left (`left: 5%`, centered vertically)

### 3.4 — Meta Search Panel (Progress → Report → PDF List → Approve → OCR)

```
┌──────────────────────────────────────────────────────────┐
│ ≡ Meta Search                            [_] [□] [×]    │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ── Active Search ─────────────────────────────────────  │
│                                                          │
│  Query: "attention mechanism survey 2024"                │
│  ┌──────────────────────────────────────────────────┐   │
│  │ ● Query enriched: 3 alternate queries generated   │   │
│  │ ● Searching 8 engines...                         │   │
│  │ ● Found 142 results across academic sources      │   │
│  │ ✓ Downloaded 8 PDFs (2 duplicates skipped)        │   │
│  │ ● Rendered 6 web pages (Lightpanda)              │   │
│  │ ● Extracting text from sources...                │   │
│  │ ● Analyzing paper 4/8: "Attention Is All You..."  │   │
│  │ ██████████████░░░░░░░░  55%                      │   │
│  └──────────────────────────────────────────────────┘   │
│                                                          │
│  ── Previous Searches ────────────────────────────────  │
│                                                          │
│  "deep learning optimization"   5 papers · 2h ago  [▶]  │
│  "Rust async patterns"          3 papers · 1d ago   [▶]  │
│                                                          │
└──────────────────────────────────────────────────────────┘

─── After search completes ───

┌──────────────────────────────────────────────────────────┐
│ ≡ Meta Search: "attention mechanism..."  [_] [□] [×]    │
│  ← All searches                                          │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ## Synthesis                                            │
│  This survey examines attention mechanisms in            │
│  transformer architectures, covering multi-head...       │
│                                                          │
│  ## Cross-Cutting Themes                                 │
│  1. Multi-head optimization techniques...                │
│  2. Sparse attention patterns reduce O(n²)...            │
│  ...                                                     │
│                                                          │
│  ## Papers (8 analyzed, ranked by relevance)             │
│                                                          │
│  ╔══════════════════════════════════════════════════╗    │
│  ║ 1. "Attention Is All You Need"          0.95    ║    │
│  ║    Vaswani et al., 2017                          ║    │
│  ║    Key: Introduced scaled dot-product attention  ║    │
│  ║    [📄 Open PDF] [📋 Copy citation]              ║    │
│  ╠══════════════════════════════════════════════════╣    │
│  ║ 2. "Linformer: Linear Attention"       0.92     ║    │
│  ║    ...                                          ║    │
│  ╚══════════════════════════════════════════════════╝    │
│                                                          │
│  ──────────────────────────────────────────────────────  │
│                                                          │
│  [✓ Approve All & Add to Knowledge Base]                 │
│  [→ Send PDFs to OCR for Text Extraction]                │
│  [📄 Open All PDFs]                                      │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

**Details:**

**A. Active Search Progress**
- Real-time progress log from meta_search subprocess
- Each pipeline step gets a log line with running status indicator
- Progress bar for paper analysis (current paper / total)
- Auto-scroll log to bottom

**B. Search History**
- Previous searches listed with query, paper count, timestamp
- Click to view cached report
- Reports and PDFs persist on disk in `reports/` and `downloads/`

**C. Report View (after search completes)**
- Full synthesis report rendered via `marked`
- Paper list as cards with: rank, title, authors, year, relevance score, key findings
- Each paper card has action buttons:
  - `📄 Open PDF` — opens in Tauri's native PDF viewer or system default app via `shell.open()`
  - `📋 Copy citation` — copies formatted citation to clipboard

**D. Action Buttons (bottom of report)**
- **Approve All & Add to KB** — ingests synthesis + all paper analyses into memory pipeline; creates knowledge graph nodes for each paper, stores in SurrealDB and Qdrant
- **Send PDFs to OCR** — sends each downloaded PDF to the OCR pipeline; opens/updates OCR panel with the queued files
- **Open All PDFs** — opens all downloaded PDFs in system viewer

**E. Panel Chrome**
- Default size: 600×550px
- Default position: center-right (`right: 5%`, centered vertically)

### 3.5 — Knowledge Base Panel (Graph Visualization + Management)

```
┌──────────────────────────────────────────────────────────┐
│ ≡ Knowledge Base                          [_] [□] [×]   │
├──────────────────────────────────────────────────────────┤
│                                                          │
│  ┌─ Tabs ──────────────────────────────────────────────┐ │
│  │ [📊 Graph]  [📋 Nodes]  [🔗 Edges]  [📁 Files]     │ │
│  └─────────────────────────────────────────────────────┘ │
│                                                          │
│  ── Tab: Graph ────────────────────────────────────────  │
│                                                          │
│  ┌──────────────────────────────────────────────────┐   │
│  │                                                  │   │
│  │              Canvas / SVG Force Graph            │   │
│  │                                                  │   │
│  │     [Sherpa]────USES───▶[Whisper]                │   │
│  │        │                    │                    │   │
│  │     WORKS_ON            SUPPORTS                 │   │
│  │        │                    │                    │   │
│  │        ▼                    ▼                    │   │
│  │     [User]◀──OWNS───[Raspberry Pi]              │   │
│  │        │                                         │   │
│  │     PREFERS                                      │   │
│  │        │                                         │   │
│  │        ▼                                         │   │
│  │     [Rust]                                       │   │
│  │                                                  │   │
│  └──────────────────────────────────────────────────┘   │
│                                                          │
│  Zoom: 100%  ·  Filter: [All ▾]  ·  Depth: [2 ▾]       │
│  ─────────────────────────────────────────────────────   │
│  Selected: [User]                                        │
│  Type: Person  ·  Created: 2026-07-20                    │
│  Relations: PREFERS→Rust, WORKS_ON→Voice Assistant, ... │
│  ─────────────────────────────────────────────────────   │
│  Graph data is managed by approved document ingestion.   │
│                                                          │
│  ── Tab: Nodes ───────────────────────────────────────  │
│                                                          │
│  ╔══════════════════════════════════════════════════╗    │
│  ║  🔍 Search nodes...                    [Type ▾] ║    │
│  ╠══════════╤═════════╤══════════╤══════╤══════════╣    │
│  ║ Name     │ Type    │ Edges    │ Created          ║    │
│  ╠══════════╪═════════╪══════════╪══════════════════╣    │
│  ║ User     │ Person  │ 12       │ 7/20             ║    │
│  ║ Rust     │ Concept │ 5        │ 7/18             ║    │
│  ║ Sherpa   │ Tool    │ 8        │ 7/15             ║    │
│  ║ Whisper  │ Tool    │ 3        │ 7/12             ║    │
│  ║ paper.pdf│ Doc     │ 1        │ 7/25             ║    │
│  ╚══════════╧═════════╧══════════╧══════╧══════════╝    │
│                                                          │
│  ── Tab: Edges ───────────────────────────────────────  │
│                                                          │
│  ╔══════════════════════════════════════════════════╗    │
│  ║ From      │ Relation   │ To        │ Confidence   ║    │
│  ╠═══════════╪════════════╪═══════════╪══════════════╣    │
│  ║ User      │ PREFERS    │ Rust      │ 0.95         ║    │
│  ║ User      │ WORKS_ON   │ Voice Asst│ 0.90         ║    │
│  ║ Sherpa    │ SUPPORTS   │ Whisper   │ 0.85         ║    │
│  ╚═══════════╧════════════╧═══════════╧══════════════╝    │
│                                                          │
│  ── Tab: Files ───────────────────────────────────────  │
│                                                          │
│  ╔══════════════════════════════════════════════════╗    │
│  ║ 📄 paper.pdf          45 KB  ·  OCR ✓  ·  7/25  [✗] ║    │
│  ║ 📄 thesis.pdf        120 KB  ·  OCR ✓  ·  7/24  [✗] ║    │
│  ║ 📄 report.md           8 KB  ·  Search  ·  7/23  [✗] ║    │
│  ║ 🖼 diagram.png        25 KB  ·  (none)  ·  7/22  [✗] ║    │
│  ╚══════════════════════════════════════════════════╝    │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

**Details:**

**A. Graph Tab (Canvas-based force-directed layout)**
- **Rendering**: HTML5 Canvas with simple force-directed layout (Coulomb repulsion + spring attraction)
- **Nodes**: Circles with labels, colored by type (Person=blue, Project=green, Tool=orange, Concept=purple, Document=gray)
- **Edges**: Lines with arrowheads, labeled with relation type
- **Interaction**: 
  - Click node → select it, highlight neighbors, show detail panel
  - Drag node → reposition, physics re-settles
  - Scroll to zoom (canvas transform)
  - Hover edge → highlight and show relation label
- **Controls below graph**:
  - Zoom slider
  - Type filter dropdown (show only certain node types)
  - Depth slider (how many hops from selected node to show)
  - No manual graph mutation; approved ingestion owns nodes and edges

**B. Nodes Tab (Table view)**
- Searchable/filterable table of all graph nodes
- Columns: Name, Type, Edge Count, Created Date
- Type filter dropdown
- Click row → select node, jump to Graph tab
- Read-only

**C. Edges Tab (Table view)**
- Table of all relationships
- Columns: From, Relation, To, Confidence
- Read-only

**D. Files Tab (Table view)**
- Lists all ingested files (PDFs, markdown, images)
- Shows: filename, size, source (OCR/Search), date, actions
- Actions: open file, delete from KB, re-process

**E. Panel Chrome**
- Default size: 700×550px
- Default position: center of screen

---

## 4. Panel System Implementation

### 4.1 — `src/ui/panel-system.js` (Core Framework)

```javascript
/**
 * PanelSystem — manages all draggable panels.
 *
 * Panels are identified by a unique `panelId` string.
 * Only one instance of each panel type exists.
 * The Status Panel is special: fixed top-left, no close, no drag.
 */

class Panel {
  constructor(id, title, opts = {}) {
    this.id = id;
    this.title = title;
    this.el = null;           // root DOM element
    this.headerEl = null;     // drag handle
    this.bodyEl = null;       // content container
    this.isVisible = false;
    this.isMinimized = false;
    this.isMaximized = false;
    this.x = opts.x ?? 100;
    this.y = opts.y ?? 100;
    this.width = opts.width ?? 500;
    this.height = opts.height ?? 400;
    this.minWidth = opts.minWidth ?? 300;
    this.minHeight = opts.minHeight ?? 200;
    this.fixed = opts.fixed ?? false;   // true for status panel
    this.closable = opts.closable ?? true;
    this.resizable = opts.resizable ?? true;
    this.zIndex = 100;
    this.preMaximize = null; // saved state before maximize

    this._dragState = null;
    this._resizeState = null;
  }

  // Create DOM, attach events, inject into document
  mount(container) { /* ... */ }

  // Set inner content (HTML or DOM nodes)
  setContent(html) { /* ... */ }

  // Visibility
  show()  { this.el.style.display = ''; this.isVisible = true; }
  hide()  { this.el.style.display = 'none'; this.isVisible = false; }

  // Minimize: collapse to titlebar only
  toggleMinimize() { /* ... */ }

  // Maximize: fill viewport (below status panel if it exists)
  toggleMaximize() { /* ... */ }

  // Position
  moveTo(x, y) { /* clamped to viewport */ }

  // Resize
  resizeTo(w, h) { /* clamped to constraints */ }

  // Bring to front (z-index)
  focus() { /* increment global z counter */ }

  // Destroy
  destroy() { this.el.remove(); }
}

// Singleton registry
export const panels = new Map();

export function createPanel(id, title, opts) {
  if (panels.has(id)) {
    const existing = panels.get(id);
    existing.show();
    existing.focus();
    return existing;
  }
  const panel = new Panel(id, title, opts);
  panel.mount(document.body);
  panels.set(id, panel);
  return panel;
}

export function getPanel(id) { return panels.get(id); }
export function removePanel(id) {
  const p = panels.get(id);
  if (p) { p.destroy(); panels.delete(id); }
}
```

### 4.2 — Drag & Resize Implementation (`src/lib/drag-resize.js`)

```
Drag:
  mousedown on .panel-header → record offset (mouseX - panelLeft, mouseY - panelTop)
  mousemove on document → panel.moveTo(mouseX - offsetX, mouseY - offsetY)
  mouseup → release

Resize:
  mousedown on .panel-resize-handle (bottom-right 8px corner)
  mousemove → panel.resizeTo(mouseX - panelLeft, mouseY - panelTop)
  mouseup → release

Constraints:
  - Min: minWidth × minHeight
  - Max: window.innerWidth - 20, window.innerHeight - 20
  - Left edge cannot go < 0 (except partial off-screen allowed)
  - Top edge cannot go < 40 (below status panel when it's expanded)
```

### 4.3 — CSS Design Tokens (`styles/tokens.css`)

```css
:root {
  /* ── Colors ─────────────────────────────── */
  --bg-primary:      #161620;     /* main background */
  --bg-secondary:    #1e1e2a;     /* panel body */
  --bg-tertiary:     #282836;     /* input fields */
  --bg-hover:        #323244;     /* hover state */

  --border-default:  rgba(255,255,255,0.06);
  --border-active:   rgba(255,255,255,0.12);

  --text-primary:    #d1d5db;     /* body text */
  --text-secondary:  #9ca3af;     /* dim text */
  --text-muted:      rgba(255,255,255,0.25);
  --text-accent:     #7fd1ff;     /* highlights, links */

  --accent-blue:     rgba(59,130,246,0.5);
  --accent-green:    rgba(34,197,94,0.5);
  --accent-red:      rgba(239,68,68,0.6);
  --accent-yellow:   rgba(234,179,8,0.5);
  --accent-purple:   rgba(168,85,247,0.5);

  --user-bubble:     rgba(59,130,246,0.28);
  --user-border:     rgba(59,130,246,0.18);
  --model-bubble:    rgba(255,255,255,0.04);
  --model-border:    rgba(255,255,255,0.06);

  /* ── Spacing ─────────────────────────────── */
  --space-xs:        4px;
  --space-sm:        8px;
  --space-md:        12px;
  --space-lg:        16px;
  --space-xl:        24px;

  /* ── Typography ──────────────────────────── */
  --font-mono:       'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
  --font-sans:       system-ui, -apple-system, sans-serif;
  --font-size-xs:    11px;
  --font-size-sm:    12px;
  --font-size-md:    13px;
  --font-size-lg:    14px;
  --line-height:     1.55;

  /* ── Panel ───────────────────────────────── */
  --panel-bg:        rgba(22,22,32,0.92);
  --panel-header-h:  32px;
  --panel-radius:    8px;
  --panel-shadow:    0 8px 32px rgba(0,0,0,0.45);
  --panel-header-bg: rgba(255,255,255,0.03);

  /* ── Z-Index ─────────────────────────────── */
  --z-canvas:        0;
  --z-panel:         100;
  --z-panel-active:  200;
  --z-sidebar:       300;
  --z-tooltip:       1000;
  --z-modal:         2000;

  /* ── Transitions ─────────────────────────── */
  --transition-fast: 0.12s ease;
  --transition-norm: 0.2s ease;

  /* ── Scrollbar ───────────────────────────── */
  --scrollbar-width: 5px;
  --scrollbar-thumb: rgba(255,255,255,0.08);
}
```

### 4.4 — CSS Panel Chrome (`styles/panel.css`)

```css
.panel {
  position: fixed;
  background: var(--panel-bg);
  border: 1px solid var(--border-default);
  border-radius: var(--panel-radius);
  box-shadow: var(--panel-shadow);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-width: var(--min-width, 300px);
  min-height: var(--min-height, 200px);
  transition: opacity var(--transition-fast);
}

.panel-header {
  height: var(--panel-header-h);
  background: var(--panel-header-bg);
  border-bottom: 1px solid var(--border-default);
  display: flex;
  align-items: center;
  padding: 0 var(--space-sm) 0 var(--space-md);
  cursor: grab;
  user-select: none;
  flex-shrink: 0;
}
.panel-header:active { cursor: grabbing; }

.panel-title {
  flex: 1;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.panel-actions {
  display: flex;
  gap: 2px;
}

.panel-btn {
  width: 24px; height: 24px;
  border: none; background: none; color: var(--text-muted);
  cursor: pointer; font-size: 14px; border-radius: 4px;
  display: flex; align-items: center; justify-content: center;
  transition: background var(--transition-fast), color var(--transition-fast);
}
.panel-btn:hover { background: rgba(255,255,255,0.08); color: var(--text-primary); }
.panel-btn.close:hover { background: rgba(239,68,68,0.3); color: #fca5a5; }

.panel-body {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: var(--space-md);
  font-size: var(--font-size-md);
  line-height: var(--line-height);
  color: var(--text-primary);
}

/* Resize handles */
.panel-resize-handle {
  position: absolute;
  bottom: 0; right: 0;
  width: 12px; height: 12px;
  cursor: nwse-resize;
  z-index: 10;
}
.panel-resize-handle::after {
  content: '';
  position: absolute;
  bottom: 2px; right: 2px;
  width: 0; height: 0;
  border-style: solid;
  border-width: 0 0 8px 8px;
  border-color: transparent transparent var(--border-active) transparent;
}

/* Minimized state */
.panel.minimized {
  height: var(--panel-header-h) !important;
  overflow: hidden;
}
.panel.minimized .panel-body,
.panel.minimized .panel-resize-handle { display: none; }

/* Status panel — fixed, non-draggable */
.panel.status-panel {
  top: 8px; left: 8px;
  width: 340px;
  max-height: 60vh;
  z-index: var(--z-panel);
}
.panel.status-panel .panel-header { cursor: default; }
.panel.status-panel .panel-actions .close { display: none; }

/* Chat panel */
.panel.chat-panel {
  /* default: positioned programmatically */
}

/* Panel sidebar toolbar */
.panel-sidebar {
  position: fixed;
  right: 12px;
  top: 50%;
  transform: translateY(-50%);
  z-index: var(--z-sidebar);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.panel-toggle-btn {
  width: 36px; height: 36px;
  border-radius: 8px;
  background: var(--panel-bg);
  border: 1px solid var(--border-default);
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 16px;
  display: flex; align-items: center; justify-content: center;
  transition: background var(--transition-fast), border-color var(--transition-fast);
  position: relative;
}
.panel-toggle-btn:hover { background: var(--bg-hover); border-color: var(--border-active); }
.panel-toggle-btn.active { border-color: var(--accent-blue); background: rgba(59,130,246,0.15); }

/* Badge for pending updates */
.panel-toggle-btn .badge {
  position: absolute;
  top: -4px; right: -4px;
  width: 10px; height: 10px;
  background: var(--accent-blue);
  border-radius: 50%;
  border: 2px solid var(--bg-primary);
}
```

### 4.5 — Sidebar (`src/ui/sidebar.js`)

```javascript
// Right-edge vertical button bar to toggle panels
// Each button represents one panel type:
//   💬 Chat    — toggles chat panel
//   📄 OCR     — toggles OCR panel  
//   🔍 Search  — toggles search panel
//   🧠 KB      — toggles knowledge base panel
//
// Active panel → button gets .active class (blue border)
// Panel has pending update → button gets .badge dot
// Status panel is NOT in sidebar (always visible, toggled via its own header)
```

---

## 5. Chat History & Session Management

### 5.1 — Data Model

```javascript
// In-memory (backed by SurrealDB once memory system is ready)
const chatSessions = new Map();  // sessionId → ChatSession

class ChatSession {
  constructor() {
    this.id = crypto.randomUUID();
    this.title = 'New Chat';
    this.messages = [];   // [{ role, content, timestamp }]
    this.createdAt = Date.now();
    this.updatedAt = Date.now();
    this.isActive = false;
  }
}
```

### 5.2 — Operations

| Operation | Behavior |
|-----------|----------|
| **New Chat** | Create `ChatSession`, set as active, clear message area, reset input |
| **Switch Chat** | Save current session state, load selected session's messages, scroll to bottom |
| **Delete Chat** | Remove from `chatSessions`, if active → switch to newest remaining or create new |
| **Auto-title** | After first user+assistant exchange, generate title from first user message (first 40 chars) |
| **Persist** | On session switch/close, serialize to SurrealDB; on app start, load all sessions |

### 5.3 — Integration with Memory System

- When a chat session is active, the memory pipeline uses its `sessionId` for conversation storage
- Chat history from SurrealDB is automatically available for retrieval
- After each turn, `process_turn()` stores the exchange with the session ID

---

## 6. Markdown Rendering System

### 6.1 — `src/lib/markdown.js`

```javascript
import { marked } from 'marked';

// Configure marked for our use case
marked.setOptions({
  breaks: true,        // single newline → <br> (chat-friendly)
  gfm: true,           // GitHub Flavored Markdown
});

// Custom renderer for chat-specific styling
const renderer = new marked.Renderer();

// Code blocks get copy button
renderer.code = function(code, language) {
  const lang = language ? `<span class="code-lang">${language}</span>` : '';
  const escaped = code
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
  return `<div class="code-block">
    <div class="code-header">${lang}<button class="copy-btn" data-code="${encodeURIComponent(code)}">Copy</button></div>
    <pre><code class="language-${language || ''}">${escaped}</code></pre>
  </div>`;
};

// Links open externally (in Tauri, opens in system browser)
renderer.link = function(href, title, text) {
  return `<a href="${href}" title="${title || ''}" target="_blank" rel="noopener">${text}</a>`;
};

export function renderMarkdown(text) {
  return marked.parse(text, { renderer });
}

export function renderInline(text) {
  return marked.parseInline(text);
}
```

### 6.2 — Code Block Copy Handler

```javascript
// Global delegate for copy buttons
document.addEventListener('click', (e) => {
  const btn = e.target.closest('.copy-btn');
  if (!btn) return;
  const code = decodeURIComponent(btn.dataset.code);
  navigator.clipboard.writeText(code).then(() => {
    btn.textContent = 'Copied!';
    setTimeout(() => btn.textContent = 'Copy', 1500);
  });
});
```

---

## 7. PDF Opener Integration (Tauri)

### 7.1 — Using Tauri Shell Plugin

```javascript
// src/lib/ipc.js

import { open } from '@tauri-apps/plugin-shell';

/**
 * Open a PDF in the system default application.
 * Falls back to Tauri's shell open.
 */
export async function openPdf(filePath) {
  try {
    await open(filePath);
  } catch (e) {
    console.error('Failed to open PDF:', e);
  }
}
```

Tauri capability for shell needs to be added in `src-tauri/capabilities/default.json`:

```json
{
  "permissions": [
    "shell:allow-open"
  ]
}
```

And `@tauri-apps/plugin-shell` needs to be added:
```bash
npm install @tauri-apps/plugin-shell
```

In `src-tauri/Cargo.toml`:
```toml
tauri-plugin-shell = "2"
```

In `src-tauri/src/lib.rs`:
```rust
.plugin(tauri_plugin_shell::init())
```

### 7.2 — Fallback for Browser Dev Mode

```javascript
export async function openPdf(filePath) {
  if (window.__TAURI_INTERNALS__) {
    const { open } = await import('@tauri-apps/plugin-shell');
    await open(filePath);
  } else {
    // Browser dev mode: open in new tab (works for localhost-served PDFs)
    window.open(filePath, '_blank');
  }
}
```

---

## 8. JS File Refactoring Plan

### 8.1 — Current `main.js` → Split Into

```
main.js (new, ~100 lines)
  - import modules
  - boot PixiJS + Live2D model
  - mount status panel
  - mount sidebar
  - init animation loop
  - export globals (via window.__app)

src/ui/panel-system.js   (~200 lines)  Panel class + registry
src/ui/status-panel.js   (~120 lines)  Service health + log feed
src/ui/chat-panel.js     (~300 lines)  Chat UI, markdown rendering, history
src/ui/ocr-panel.js      (~180 lines)  Upload, queue, preview, approve
src/ui/search-panel.js   (~220 lines)  Progress, report, PDF list, approve
src/ui/kb-panel.js       (~350 lines)  Graph visualization + 3 tables
src/ui/sidebar.js        (~50 lines)   Panel toggle buttons

src/lib/ipc.js           (~30 lines)   Tauri IPC wrapper
src/lib/markdown.js      (~40 lines)   marked config + renderer
src/lib/drag-resize.js   (~80 lines)   Drag + resize handlers

src/tts-player.js        (unchanged)

styles/tokens.css        (~50 lines)   CSS custom properties
styles/panel.css         (~120 lines)  Panel chrome + resize + states
styles/status.css        (~80 lines)   Status panel specific
styles/chat.css          (~120 lines)  Chat bubbles + markdown + sidebar
styles/ocr.css           (~60 lines)   Upload zone + queue items
styles/search.css        (~80 lines)   Progress log + paper cards
styles/kb.css            (~100 lines)  Graph + tables + tabs
styles/sidebar.css       (~30 lines)   Toggle button bar
```

---

## 9. Detailed Implementation Order

### Phase 1 — CSS Foundation (1 hour)
```
1. Create styles/tokens.css        — CSS variables, design tokens
2. Create styles/panel.css         — Panel chrome, drag/resize states
3. Create styles/scrollbar.css     — Global scrollbar (extract from #chat-messages)
4. Adapt index.html                — Link new CSS files, strip inline styles
5. Verify: No visual regressions at this stage
```

### Phase 2 — Panel System (1.5 hours)
```
6. Create src/lib/drag-resize.js   — Drag + resize primitives
7. Create src/ui/panel-system.js   — Panel class + registry singleton
8. Test: Create 2-3 test panels, verify drag/resize/minimize/maximize/close
```

### Phase 3 — Status Panel (1 hour)
```
9. Create styles/status.css        — Service indicators, log feed, minimized
10. Create src/ui/status-panel.js  — Health checks (Ollama ping, etc.), log feed
11. Integrate: Mount status panel on app start
12. Move expression buttons into status panel (collapsible section)
```

### Phase 4 — Chat Panel (2 hours)
```
13. Create styles/chat.css         — Bubbles, sidebar, input, markdown
14. Create src/lib/markdown.js     — marked renderer with code copy
15. Create src/ui/chat-panel.js    — Full chat UI with history sidebar
16. Port chatSendMessage() from main.js → chat-panel.js
17. Remove old #chatbox from index.html
18. Test: Full chat flow with markdown, multiple sessions
```

### Phase 5 — OCR Panel (1.5 hours)
```
19. Create styles/ocr.css          — Upload zone, queue, preview
20. Create src/ui/ocr-panel.js     — Drag-drop upload, queue, markdown preview
21. Wire Tauri command for OCR processing (invoke ocr_helper subprocess)
22. Add approve flow (call memory ingestion pipeline)
23. Test: Upload PDF, view result, approve to KB
```

### Phase 6 — Meta Search Panel (1.5 hours)
```
24. Create styles/search.css       — Progress log, report, paper cards
25. Create src/ui/search-panel.js  — Real-time progress, report view, controls
26. Wire Tauri command for meta_search
27. Add PDF opener integration
28. Add "Send to OCR" button logic
29. Test: Run search, view report, open PDF, approve to KB
```

### Phase 7 — Knowledge Base Panel (2 hours)
```
30. Create styles/kb.css           — Graph canvas, tables, tabs
31. Create src/ui/kb-panel.js      — Graph viz (Canvas), 3 table tabs
32. Wire to SurrealDB/Qdrant for live data
33. Add node/edge/file deletion with confirmation
34. Add "add node" / "add edge" manual entry
35. Test: View graph, delete nodes, add manual entries
```

### Phase 8 — Sidebar + Polish (1 hour)
```
36. Create styles/sidebar.css      — Toggle button bar
37. Create src/ui/sidebar.js       — Panel toggle + badge system
38. Add keyboard shortcuts: Ctrl+C=toggle chat, Ctrl+O=OCR, etc.
39. Final cleanup: Remove old #hud, #chatbox, dead CSS from index.html
```

### Phase 9 — Panel State Persistence (30 min)
```
40. Save/restore panel positions to localStorage
41. Save/restore which panels were open
42. Restore on app restart
```

**Total: ~12 hours**

---

## 10. Edge Cases & Error Handling

| Scenario | Handling |
|----------|----------|
| Panel dragged off-screen | Clamp to viewport bounds (min 20px visible) |
| Resize below minimum | Enforce minWidth/minHeight, show resize cursor change |
| Multiple panels open, overlapping | Auto z-index management: click-to-front |
| OCR processing fails mid-way | Show error in queue item, [Retry] button, partial text preserved |
| meta_search times out | Show partial results with warning banner, allow re-run |
| Knowledge graph >500 nodes | Canvas renders only visible nodes; table view paginates |
| Panel restore after close | Content cached, panel re-mounts at last position |
| Session switch while streaming | Block switch, show "waiting for response..." tooltip |
| Delete active chat session | Switch to most recent remaining, or create new if none |
| PDF file deleted from disk after KB ingest | File tab shows "File missing" badge, keep metadata |
| Embedding model offline during search | Searches continue, skip vector embedding, show warning |
| SurrealDB connection lost during KB view | Show cached data, retry indicator, auto-reconnect |
| Window resize while panels open | Panels maintain proportional position, clamp to new bounds |

---

## 11. NPM Dependencies to Add

```json
{
  "dependencies": {
    "@tauri-apps/plugin-shell": "^2.0.0"
  },
  "devDependencies": {
    // No additional dev deps needed — we use vanilla DOM, Canvas, marked (already present)
  }
}
```

Existing `marked` (^18.0.6) is sufficient for all markdown rendering.

---

## 12. Tauri Capabilities Update

In `src-tauri/capabilities/default.json`:
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "shell:allow-open"
  ]
}
```

In `src-tauri/Cargo.toml`:
```toml
tauri-plugin-shell = "2"
```

In `src-tauri/src/lib.rs`:
```rust
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        // ... rest
}
```

---

## 13. File Manifest Summary

```
NEW FILES (22):
├── src/
│   ├── lib/
│   │   ├── ipc.js
│   │   ├── markdown.js
│   │   └── drag-resize.js
│   └── ui/
│       ├── panel-system.js
│       ├── status-panel.js
│       ├── chat-panel.js
│       ├── ocr-panel.js
│       ├── search-panel.js
│       ├── kb-panel.js
│       └── sidebar.js
├── styles/
│   ├── tokens.css
│   ├── panel.css
│   ├── status.css
│   ├── chat.css
│   ├── ocr.css
│   ├── search.css
│   ├── kb.css
│   └── sidebar.css

MODIFIED FILES (5):
├── index.html                   # Strip old CSS/HTML, link new styles, minimal DOM
├── src/main.js                  # Slim to bootstrapper (~100 lines)
├── package.json                 # Add @tauri-apps/plugin-shell
├── src-tauri/Cargo.toml         # Add tauri-plugin-shell
├── src-tauri/capabilities/default.json  # Add shell:allow-open
├── src-tauri/src/lib.rs         # Add plugin init

DELETED (none — old code ported, not removed):

UNCHANGED (1):
├── src/tts-player.js
```
