# ALICE — Full Electron + Rust Backend Migration Plan

## 1. Objective

Make Electron/Chromium the fully functional high-performance desktop target while
retaining the existing Rust memory, knowledge graph, OCR, search, TTS, database,
and process-management implementation.

The completed Electron application must have feature and data compatibility with
the current Tauri application:

- every existing Rust command works from Electron;
- chat streaming, progress events, and cancellation behave identically;
- existing SurrealDB and Qdrant data remains compatible;
- OCR, Meta Search, Quick Extract, knowledge ingestion, graph cleanup, document
  deletion, chat history, and TTS work in packaged builds;
- Electron retains the observed stable Chromium/WebGL 2 rendering performance;
- Tauri remains buildable only as the reference host until Electron passes all
  parity and production gates, then is removed from the active application.

This is not a rewrite of the Rust backend in Node.js. It is a transport and
packaging migration around one shared Rust core.

### Approved transition lifecycle

```text
1. Shared Rust backend + existing Tauri + Electron
2. Tauri/Electron parity, persistence, packaging, and production tests
3. Electron + Rust backend becomes the production application
4. Tag the final Tauri reference, then remove Tauri code and dependencies
```

Tauri is a temporary verification oracle, not a permanent second product.

### Implementation status — Phase 1 extraction complete

Completed in the first Phase 1 slice:

- created the Tauri-free `crates/alice-core` crate;
- moved `MemoryConfig`, adaptive Ollama context selection, `ProcessRegistry`,
  cancellation, and process reaping into `alice-core`;
- added the stable 29-command/four-event catalog and structured backend error
  envelope;
- added `EventSink`, `ResourceLocator`, and `BackendFoundation`;
- connected Tauri to `BackendFoundation` through a temporary
  `TauriEventSink`;
- removed `tauri::AppHandle` from extraction/ingestion progress logic;
- removed `AppHandle.path()` from OCR and Meta Search helper resolution;
- verified the Tauri-free core has no Tauri dependency;
- passed the new core tests and existing Tauri Rust test suite.

Completed in the second Phase 1 slice:

- moved the SurrealDB and Qdrant adapters into `alice-core`;
- moved the complete memory, graph, retrieval, extraction, ingestion, document
  store, and graph-cleanup module set into `alice-core`;
- moved Quick Extract, OCR, and Meta Search helpers into `alice-core`;
- added the first real `BackendApi` operations and stable DTOs for
  `check_db_ready`, `get_kb_data`, `get_files`, and `get_document_text`;
- converted those four Tauri commands into thin transport wrappers;
- replaced remaining Tauri access to private memory-manager fields with narrow
  core methods;
- moved the Piper/sherpa-onnx TTS engine and asynchronous synthesis service into
  `alice-core`, leaving two thin Tauri command wrappers;
- added the runtime-neutral `StreamSink` contract;
- verified `alice-core` still contains no Tauri, Wry, or WebKit dependency;
- passed 22 core tests, 2 Tauri adapter tests, frontend tests, and the production
  Vite build.

The next Phase 1 slice moves Group A persisted queries (chat/search/document
search) and then database mutations behind `BackendApi`. Process-heavy OCR,
search, cancellation, chat streaming, and graph ingestion follow after those
request/response operations.

Completed in the third Phase 1 slice:

- moved `load_chat_sessions`, `get_search_history`,
  `get_search_document_content`, and `search_documents` into `BackendApi`;
- added stable chat-session, chat-message, search-history, and vector-search
  response DTOs;
- preserved the search-history recovery rule that recognizes an already
  committed KB knowledge document after an interrupted status update;
- kept legacy large/NUL-containing search blobs out of history queries;
- converted all eight Group A commands into thin Tauri wrappers;
- added core contract tests for response field names, search-document grouping,
  default modes, and ingestion reconciliation;
- passed 24 core tests, 2 Tauri adapter tests, frontend tests, and the production
  Vite build.

The next slice moves Group B database mutations behind `BackendApi`, starting
with chat sessions and search-document processing state, followed by KB file
records and source-aware graph deletion.

Completed in the fourth Phase 1 slice:

- moved `save_chat_session` and `delete_chat_session` into `BackendApi`;
- moved `save_search_document_result`, `mark_search_document_ingested`, and
  `mark_search_knowledge_ingested` into `BackendApi`;
- moved `save_file_record`, `delete_file_record`, and `delete_file_nodes` into
  `BackendApi`;
- preserved approval-time vector commits, database rollback cleanup, in-memory
  document-text compatibility, Qdrant deletion, source-aware graph deletion,
  and graph persistence;
- moved document/session size limits and extraction-method validation into the
  shared core so the future Electron sidecar cannot bypass them;
- converted all Group B commands into thin Tauri wrappers;
- passed 25 core tests and 2 Tauri adapter tests, with service-dependent tests
  remaining explicitly ignored.

The next slice begins Group C: prompt enrichment, turn processing, tool-aware
message processing, consolidation, ingestion, and graph cleanup. Streaming chat
and subprocess orchestration remain separate because they require the new
runtime-neutral stream/process adapters.

Completed in the fifth Phase 1 slice:

- moved `enrich_prompt`, `process_turn`, and tool-aware `process_message` into
  `BackendApi`;
- moved `run_consolidation` and `ingest_text` into `BackendApi`;
- moved the full LLM graph analysis, conservative duplicate filtering, graph
  merge, edge deduplication, and persistence operation behind
  `analyze_and_merge_kb`;
- added stable DTOs for message processing, consolidation, graph-cleanup
  summaries, and individual merged groups;
- routed ingestion and graph-cleanup progress through the host-neutral
  `EventSink` contract;
- moved ingestion payload enforcement into the shared core;
- converted all six Group C commands into thin Tauri wrappers;
- passed 26 core tests, 2 Tauri adapter tests, frontend tests, and the production
  Vite build.

The next slice handles streaming chat with `StreamSink`, then Quick Extract and
process cancellation. OCR and Meta Search orchestration will follow using the
shared process registry and resource locator.

Completed in the sixth Phase 1 slice:

- moved Ollama chat streaming, adaptive context sizing, response validation,
  fragmented-line buffering, delta parsing, and completion signaling into
  `BackendApi`;
- added a thin `TauriChannelStreamSink` adapter while keeping the stream
  protocol host-neutral;
- added contract coverage for non-empty delta emission and malformed stream
  records;
- moved Quick Extract path validation, source-size enforcement, blocking file
  parsing, chunk counting, and its response DTO into `BackendApi`;
- added `ProcessRegistry::cancel` so process cancellation no longer exposes the
  registry's internal map to the Tauri command;
- converted `stream_chat`, `run_quick_extract`, and `cancel_process` into thin
  host adapters;
- passed 27 core tests and 2 Tauri adapter tests; the Tauri application remains
  warning-free and `alice-core` remains free of Tauri/Wry/WebKit dependencies.

The remaining Phase 1 transport extraction is concentrated in `run_ocr` and
`run_search`. Their helper-process lifecycle, search fallback, persistence, and
knowledge-document generation will move to the core before Phase 2 introduces
the framed sidecar protocol.

Completed in the seventh Phase 1 slice:

- encapsulated child registration, cancellable waiting, output collection,
  cleanup, and task completion inside `ProcessRegistry`;
- moved OCR validation, helper discovery, process execution, progress events,
  cancellation, output reading, and response DTOs into `BackendApi`;
- moved the complete research pipeline into the Tauri-free `research` module:
  SearXNG knowledge gathering, safe public-page fetching, HTML cleanup, paper
  mode helper execution, Meta Search JSON recovery, adaptive-context knowledge
  writing, deterministic fallback generation, downloaded-PDF inventory, and
  SurrealDB persistence;
- converted `run_ocr` and `run_search` into thin Tauri wrappers;
- replaced the former 1,200-line mixed Tauri/business command module with a
  331-line adapter-only module containing the same registered command surface;
- introduced the owning `BackendState` aggregate for the Phase 2 sidecar;
- passed 29 core tests, Tauri compilation, frontend tests, and the production
  Vite build.

Phase 1 code extraction is complete. Its final live-service/manual workflow gate
remains part of parity testing because Ollama, SurrealDB, Qdrant, SearXNG,
Lightpanda, OCR models, and the search helper must be running. Phase 2 can now
start without introducing any Tauri dependency into the sidecar.

### Implementation status — Phase 2 started

Completed in the first Phase 2 slice:

- created the Tauri-free `crates/alice-sidecar` binary crate;
- implemented protocol version 1 with a four-byte big-endian length prefix and
  named-field MessagePack payloads;
- enforced the 64 MiB frame limit before payload allocation;
- defined typed hello, request, response, event, stream, ping/pong, and shutdown
  messages;
- retained native MessagePack binary values for future WAV responses;
- implemented the startup handshake from the shared 29-command catalog and
  advertised binary, event, streaming, and cancellation capabilities;
- added a minimal ping/shutdown loop and structured pre-dispatch errors;
- passed framing tests for byte-by-byte fragmented input, oversize rejection,
  binary round trips, and clean EOF;
- verified the sidecar dependency tree contains no Tauri, Wry, or WebKit
  packages.

The next Phase 2 slice initializes `BackendState`, implements the command
dispatcher beginning with Group A reads and Group B mutations, and adds a Node
diagnostic client that exercises the real sidecar process.

Completed in the second Phase 2 slice:

- made the sidecar own and initialize the real `BackendState`, with an explicit
  offline construction path for deterministic protocol diagnostics;
- implemented concurrent request dispatch with a 64-request admission limit,
  an allowlist sourced from the shared command catalog, and structured overload,
  invalid-parameter, unknown-command, and backend errors;
- connected every Group A read and Group B mutation to the same `BackendApi`
  operations used by Tauri;
- connected chat streaming, OCR, Meta Search, and process cancellation early so
  request-scoped stream and event routing are exercised by real core paths;
- added MessagePack event and stream sinks that preserve the request ID as the
  operation/correlation ID;
- fixed named-field result encoding at the nested command-result boundary,
  preventing Rust DTOs from arriving in JavaScript as positional arrays;
- added clean request-task cancellation and output-writer shutdown;
- added `@msgpack/msgpack` and an executable Node diagnostic covering the
  handshake, all 29 advertised methods, ping/pong, two real offline core reads,
  and clean process shutdown;
- passed eight sidecar unit/contract tests and the end-to-end Node diagnostic.

The next Phase 2 slice connects the remaining Group C/D/E dispatcher methods,
including TTS binary output and Quick Extract, then expands the Node client to
exercise every command category and failure mode.

Completed in the third Phase 2 slice:

- connected all remaining Group C memory/graph commands, Quick Extract, and
  TTS start/stop operations;
- returned generated WAV data as a native MessagePack binary value rather than
  a JSON integer array;
- added an invariant test that invokes every advertised command route and fails
  if any of the 29 methods can fall through to `method_not_implemented`;
- expanded the offline process diagnostic across every command category:
  persisted-read shape, Quick Extract, in-memory ingestion/graph mutation,
  cancellation, TTS and database error envelopes, and the command allowlist;
- disabled sherpa native debug output because sidecar stdout is exclusively the
  framed protocol channel;
- passed 10 sidecar tests and the expanded end-to-end diagnostic.

Phase 2 is structurally complete. Live Ollama/TTS/OCR/search and database
workflows remain in the shared parity gate, but there is no longer an
undispatched command in the sidecar.

### Implementation status — Phase 3 started

Completed in the first Phase 3 slice:

- added a reusable Electron-main `SidecarManager` with binary discovery,
  spawn-without-shell, versioned handshake validation, named command allowlist,
  request IDs, per-command timeouts, a 64-request limit, stdin backpressure,
  request-scoped streams, global events, ping/pong, structured backend errors,
  pending-request cleanup, graceful shutdown, and forced termination fallback;
- extracted the JavaScript MessagePack frame encoder/decoder and added
  fragmented, coalesced, oversized, incomplete, and big-endian framing tests;
- changed the process diagnostic to exercise this production manager;
- initialized the manager from Electron main and exposed real runtime/backend
  status, invoke, and stream handlers through trusted-origin-checked IPC;
- exposed a narrow preload API without renderer Node.js access;
- added the runtime-neutral frontend `invokeStream` adapter and removed the chat
  panel's direct Tauri `Channel` import;
- staged the sidecar manager/protocol code, release Rust binary, and Piper model
  in the Electron package configuration;
- made Electron development/start/package scripts build the correct sidecar
  profile before launching or packaging;
- made renderer requests arriving during backend startup wait for the handshake
  instead of failing transiently;
- fixed packaged sidecar spawning to use the user-data directory rather than
  the read-only `app.asar` file as its working directory;
- passed the unpacked production launch smoke test and verified that forced GUI
  termination left no Electron or `alice-sidecar` process behind.

The next Phase 3 slice adds renderer-lifecycle cancellation, bounded unexpected
crash restart, backend status presentation, and automated Electron-main IPC
tests. Live-service smoke testing and packaged helper discovery follow.

Completed in the second Phase 3 slice:

- added request-level `cancel_request` protocol control and changed the Rust
  runtime to retain abortable request tasks by request ID;
- assigned every Electron request to its originating renderer and cancel all
  owned Rust work on navigation, renderer failure, disposal, or window
  destruction;
- added bounded unexpected-crash recovery with three increasing restart delays,
  a stable-runtime reset window, restart-limit failure state, protocol-failure
  restart blocking, and manual recovery;
- preserved the no-command-replay rule: pending requests are rejected on a
  crash and are never silently resubmitted after reconnection;
- exposed `starting`, `ready`, `restarting`, `failed`, `stopping`, and `stopped`
  states through main/preload, displayed them in the System Status panel, and
  added a manual **Restart backend** control;
- extracted and tested Electron IPC origin, command, argument, payload, and file
  dialog policies;
- added deterministic SidecarManager tests for crash recovery, restart limits,
  renderer-owned cancellation, and sidecar cancellation notification;
- added a non-mutating live diagnostic and verified real persisted state through
  the sidecar: 375 graph nodes, 635 edges, 4 files, 2 chat sessions, and 2 search
  history records.

Completed in the third Phase 3 slice:

- added a deterministic `electron:stage` pipeline that requires and stages the
  release sidecar, OCR helper, and Meta Search helper with executable
  permissions before Electron packaging;
- made missing resources fail early with their exact path and build command,
  and added an unpacked-package inspector for the three executables, Piper
  model, and application archive;
- added request-ID-based cancellable operations to the runtime-neutral frontend
  IPC adapter, allowing Electron to abort work before a helper PID is emitted
  while retaining Tauri's existing PID cancellation fallback;
- connected immediate cancellation to Meta Search, OCR, Quick Extract, search
  document processing, and knowledge ingestion controls;
- correlated Electron progress events with their originating request so
  concurrent long operations cannot capture another operation's helper PID;
- restricted explicit request cancellation to the renderer that owns the
  request and added IPC/manager coverage for identifier validation and
  cross-renderer cancellation denial;
- added a reversible live mutation diagnostic that saved a two-message chat
  session through the sidecar, read it back, deleted it, verified no record
  remained, and shut down cleanly;
- added resource-staging tests covering environment overrides, executable
  permissions, and actionable missing-dependency reporting.

Phase 3's connection exit gate is complete: Electron reads real persisted
state, performs and verifies a basic SurrealDB mutation, presents backend
health/recovery, and owns request lifecycle through the Rust sidecar. Phase 4
can now remove the remaining browser-preview branches and finish UI-level
runtime-neutral parity.

### Implementation status — Phase 4 started

Completed in the first Phase 4 slice:

- added runtime-neutral backend availability, runtime information, restart
  capability, service-health URL, and cancellable-stream helpers to the shared
  frontend adapter;
- removed all direct Electron preload, Tauri global, raw IPC implementation,
  and direct Ollama proxy access from feature UI modules;
- removed simulated OCR, Quick Extract, and Meta Search success paths: without
  a Rust desktop backend these features now report an explicit unavailable
  error and never create fake processed data;
- routed chat exclusively through the shared Rust streaming command in both
  desktop hosts and removed the browser-to-Ollama implementation;
- added Electron chat-stream cancellation to the send/stop control using the
  existing request ownership and Rust `cancel_request` protocol;
- routed runtime information, backend restart visibility, service-health URLs,
  native file handling, TTS generation/stop, OCR, search, ingestion, graph,
  chat persistence, and chat streaming through `src/lib/ipc.js`;
- made disabling TTS clear queued audio, stop current playback, and notify the
  shared Rust backend;
- added adapter contract tests for Electron, Tauri, and unavailable browser
  modes plus a source-level regression test that rejects host-specific globals,
  raw Ollama access, or simulated OCR/search paths in feature modules.

The next Phase 4 slice should exercise live chat/TTS/OCR/search workflows from
the Electron host, improve unavailable/degraded controls, and close any
UI-visible parity differences before declaring the Phase 4 exit gate complete.

Completed in the second Phase 4 slice:

- added a reversible live feature diagnostic covering Piper WAV generation,
  real Ollama chat streaming, request-level chat cancellation, search-start
  cancellation, SurrealDB/Qdrant-backed knowledge ingestion, and complete
  diagnostic file/node cleanup;
- verified live TTS with a valid 66,604-byte RIFF/WAVE payload and live chat
  streaming with nine correlated deltas;
- verified search cancellation after its correlated `search-started` event
  without creating a search-history record;
- ran the packaged OCR path against a six-page PDF and extracted 22,773
  characters through `deepseek-ocr`;
- fixed a critical OCR transport defect where helper stdout inherited the
  sidecar's framed stdout and was decoded as a 1.28 GB MessagePack frame;
- isolated OCR helper stdout, retained captured stderr for actionable errors,
  and verified the real OCR workflow no longer corrupts the protocol;
- made helper output cleanup deterministic on success, failure, cancellation,
  and dropped request futures, eliminating leaked `/tmp/alice_ocr_*.md` files;
- added process cleanup guards so aborting an Electron request removes its
  process-registry entry and reaps any registered helper child;
- added tests for temporary-output removal and dropped-request registry
  cleanup.

Phase 4 is complete. Every feature UI path now uses the runtime-neutral adapter,
fake browser success paths are gone, and the real Electron sidecar workflows
for chat, TTS, OCR, ingestion, and search cancellation pass. Phase 5 can focus
on distributable AppImage/Debian packaging without changing feature behavior.

## 2. Current State and Compatibility Gap

### Already working in Electron

- Shared Vite/PixiJS/Live2D frontend.
- Chromium/WebGL 2 rendering at stable fullscreen frame rate.
- Sandboxed BrowserWindow with context isolation and no renderer Node.js access.
- Secure preload for native file selection and opening local files.
- Production loopback static server.
- Same-origin proxies for Ollama streaming and service health checks.
- AppImage and Debian packaging.
- Runtime-aware frontend IPC abstraction.

### Remaining compatibility gap

The business implementation is no longer tied to Tauri: all 29 Tauri commands
are thin adapters over `alice-core`, and the sidecar already dispatches the
complete basic read/mutation groups plus several process-heavy operations.

Electron still enters browser preview/fallback paths because its main process
does not yet own a production `SidecarManager` and the sidecar dispatcher does
not yet connect every command category. The remaining bridge work is:

- connect the remaining memory/graph, Quick Extract, and TTS methods;
- test native binary and long-lived streaming behavior end to end;
- implement Electron lifecycle, backpressure, timeout, crash, and restart
  handling;
- expose the narrow backend API from preload and route `src/lib/ipc.js` through
  it.

### Current Rust command surface

The migration must preserve all 29 registered commands:

| Domain | Commands |
|---|---|
| TTS | `generate_tts`, `stop_tts` |
| Chat and memory | `enrich_prompt`, `process_turn`, `process_message`, `stream_chat` |
| Knowledge graph | `get_kb_data`, `analyze_and_merge_kb`, `run_consolidation` |
| OCR and research | `run_ocr`, `run_search`, `cancel_process` |
| Search history | `get_search_history`, `get_search_document_content`, `save_search_document_result`, `mark_search_document_ingested`, `mark_search_knowledge_ingested` |
| Ingestion and files | `ingest_text`, `save_file_record`, `get_files`, `get_document_text`, `delete_file_record`, `delete_file_nodes`, `run_quick_extract`, `search_documents` |
| Chat sessions | `save_chat_session`, `load_chat_sessions`, `delete_chat_session` |
| Health | `check_db_ready` |

Existing event compatibility must also be preserved:

| Event | Use |
|---|---|
| `ingest-progress` | Knowledge extraction and graph ingestion progress |
| `kb-cleanup-progress` | AI graph analysis and merge progress |
| `ocr-started` | Supplies the cancellable OCR process ID |
| `search-started` | Supplies the cancellable search process ID |
| Chat channel events | `delta` and `done` records from `stream_chat` |

## 3. Target Architecture

```text
┌───────────────────────────────────────────────────────────────┐
│ Electron/Chromium                                             │
│                                                               │
│  Renderer: shared Vite + PixiJS + Live2D UI                  │
│       │                                                       │
│       │ narrow contextBridge API                              │
│       ▼                                                       │
│  Preload                                                     │
│       │ Electron IPC                                          │
│       ▼                                                       │
│  Main process                                                │
│  ├─ validates sender, command, payload, and limits            │
│  ├─ SidecarManager: lifecycle, requests, streams, events      │
│  ├─ native dialog/shell operations                            │
│  └─ static assets + local service proxies                     │
│       │                                                       │
│       │ framed MessagePack over stdin/stdout                  │
│       ▼                                                       │
│  alice-backend Rust sidecar                                  │
│  ├─ BackendApi command dispatcher                             │
│  ├─ MemoryManager + knowledge graph                           │
│  ├─ ProcessRegistry                                           │
│  ├─ TTS engine                                                │
│  └─ OCR/search tool orchestration                             │
└───────┬───────────────────┬──────────────────┬────────────────┘
        │                   │                  │
  SurrealDB             Qdrant             Ollama
        │                                      │
     SearXNG                              OCR/Search helpers
   Lightpanda
```

The Tauri application temporarily becomes a second adapter:

```text
Tauri frontend → Tauri command wrappers → alice-core BackendApi
Electron UI    → preload/main/sidecar   → alice-core BackendApi
```

Both targets therefore exercise the same business logic instead of maintaining
two backends.

## 4. Repository Layout

Create Tauri-free Rust crates and keep the Tauri shell small:

```text
crates/
├── alice-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── api.rs              # BackendApi and command dispatch
│       ├── command.rs          # request/response DTOs
│       ├── error.rs            # stable structured errors
│       ├── events.rs           # EventSink trait and event DTOs
│       ├── resources.rs        # deterministic resource/tool paths
│       ├── db/
│       ├── memory/
│       ├── processes.rs
│       ├── tools/
│       └── tts.rs
│
└── alice-sidecar/
    ├── Cargo.toml
    └── src/
        ├── main.rs
        ├── protocol.rs         # frame codec and handshake
        ├── dispatcher.rs
        └── shutdown.rs

src-tauri/
├── Cargo.toml                  # depends on alice-core
└── src/
    ├── lib.rs                  # Tauri setup only
    └── commands.rs             # thin Tauri adapters

electron/
├── main.mjs
├── preload.cjs
├── sidecar-manager.mjs
├── sidecar-protocol.mjs
├── static-server.mjs
└── dev.mjs
```

A root Cargo workspace can be introduced after the first extraction is compiling.
`src-tauri/Cargo.toml` must remain a valid Tauri manifest throughout.

## 5. Transport Contract

### Why private pipes

The sidecar should use its inherited stdin/stdout instead of an HTTP server:

- no listening port;
- no CORS;
- no authentication token to leak;
- only the Electron parent can communicate with the child;
- shutdown is naturally detected when stdin closes;
- works for development and packaged applications;
- keeps SurrealDB, Qdrant, Ollama, SearXNG, and Lightpanda as the only network
  services.

### Framing and encoding

Use a four-byte big-endian frame length followed by MessagePack:

```text
[u32 payload length][MessagePack payload]
```

MessagePack is preferred over newline JSON because:

- `generate_tts` can return WAV bytes without expanding them into integer arrays;
- 50 MiB document payloads do not require base64;
- arbitrary text cannot break framing;
- partial pipe reads can be reassembled deterministically;
- response, stream, and event messages use one protocol.

Set a hard maximum frame size of 64 MiB initially. Reject larger frames before
allocation. Preserve the lower command-specific limits already enforced in Rust.
Use `rmp-serde` in the sidecar and `@msgpack/msgpack` in Electron main; neither
dependency is exposed to the renderer.

### Protocol messages

```text
Hello:
  { kind, protocolVersion, backendVersion, pid, methods, capabilities }

Request:
  { kind: "request", id, method, params }

Response:
  { kind: "response", id, result }
  { kind: "response", id, error: { code, message, retryable, details? } }

Event:
  { kind: "event", name, operationId?, payload }

Stream:
  { kind: "stream", requestId, event: { type: "delta", content } }
  { kind: "stream", requestId, event: { type: "done" } }

Control:
  { kind: "ping", id }
  { kind: "pong", id }
  { kind: "shutdown" }
```

Protocol version mismatch must fail startup with a clear error. Electron only
sets `backendAvailable: true` after a valid `Hello` and method-capability check.

All sidecar logging goes to stderr. Writing logs to stdout is a protocol
violation and must be covered by tests.

## 6. Rust Refactoring Strategy

### 6.1 Introduce shared application state

Replace Tauri-managed parameters with normal Rust ownership:

```text
BackendState
├─ Arc<MemoryManager>
├─ Arc<ProcessRegistry>
├─ Arc<TtsState>
├─ Arc<dyn EventSink>
└─ ResourceLocator
```

`BackendApi` owns or references `BackendState` and exposes typed operations plus
a transport dispatcher.

### 6.2 Remove Tauri from business logic

Replace each Tauri dependency as follows:

| Current dependency | Shared-core replacement |
|---|---|
| `State<'_, T>` | `Arc<T>` in `BackendState` |
| `AppHandle.emit` | `EventSink::emit` |
| `tauri::ipc::Channel` | request-scoped `StreamSink` |
| `AppHandle.path().resource_dir()` | `ResourceLocator` supplied at startup |
| Tauri async runtime | Tokio runtime owned by each host |
| String-only errors | structured `BackendError` converted at adapters |

`alice-core` must have no dependency on `tauri`, `electron`, or UI code. Add a CI
check that fails if `cargo tree -p alice-core` contains Tauri.

### 6.3 Preserve the Tauri adapter

Keep the existing `#[tauri::command]` function names and JavaScript argument
shapes. Each wrapper should:

1. obtain the shared `BackendApi` from Tauri state;
2. adapt Tauri channels/events to `StreamSink`/`EventSink`;
3. call the same core function used by the sidecar;
4. convert `BackendError` to the current string error shape.

This ensures core extraction is behavior-neutral before Electron is enabled.

### 6.4 Resource resolution

`ResourceLocator` should resolve in this order:

1. explicit command-line/resource directory supplied by the host;
2. explicit `ALICE_OCR_BIN`, `ALICE_SEARCH_BIN`, and TTS environment overrides;
3. packaged `resources/` paths;
4. development workspace paths.

Do not derive production resources from the current working directory.

### 6.5 Configuration

Use one documented precedence order:

1. sidecar command-line options supplied by Electron;
2. process environment;
3. user configuration file;
4. `.env` in development only;
5. safe defaults.

Electron should use `app.getPath("userData")` for user configuration and logs.
Secrets must not be sent to the renderer or included in sidecar log messages.

## 7. Electron Backend Integration

### 7.1 SidecarManager

Create an Electron-main-only `SidecarManager` responsible for:

- locating the development or packaged `alice-backend` binary;
- spawning it without a shell;
- passing the resource directory and sanitized environment;
- decoding fragmented/coalesced MessagePack frames;
- assigning request UUIDs;
- maintaining pending request and stream maps;
- honoring stdin backpressure and the `drain` event;
- forwarding allowlisted events to the correct renderer;
- exposing backend state: `starting`, `ready`, `degraded`, `restarting`, `failed`;
- graceful shutdown, followed by forced termination after a short deadline;
- failing all pending requests if the child exits;
- bounded automatic restart for unexpected crashes.

Recommended restart policy: at most three attempts with increasing delay. Never
silently retry a command that may have committed data. Only reconnect the
transport and require the UI/user to retry the operation.

### 7.2 Electron IPC

Add these main-process handlers:

```text
alice:backend-status
alice:invoke
alice:stream-start
alice:stream-cancel
alice:select-files
alice:open-path
```

Keep the existing trusted-origin validation. Add:

- a fixed command allowlist matching the Rust handshake;
- per-request payload limits;
- a maximum number of outstanding requests;
- command-specific timeouts;
- renderer destruction cleanup;
- structured error serialization.

Do not expose the raw child process, filesystem, resource paths, ports, or
environment through the preload.

### 7.3 Preload API

Expose a narrow immutable API:

```text
runtimeInfo()
backendStatus()
invoke(command, args)
stream(command, args, callback)
listen(event, callback)
selectFiles(options)
openPath(path)
```

`backendAvailable` must be based on the successful handshake rather than a
hard-coded constant.

### 7.4 Frontend adapter

Extend `src/lib/ipc.js` with a generic stream operation:

```text
invoke(command, args)
invokeStream(command, args, onEvent)
listenEvent(event, callback)
```

For Tauri, `invokeStream` creates a Tauri `Channel`. For Electron, it uses the
preload stream API. `chat-panel.js` must stop importing Tauri `Channel` directly.

After this change, no UI module may:

- inspect `window.__TAURI_INTERNALS__` directly;
- import Tauri APIs directly;
- mention that document processing requires Tauri.

Only `src/lib/ipc.js` is allowed to know which desktop runtime is active.

## 8. Command Migration Order

Migrate and verify commands in increasing complexity:

### Group A — Basic request/response

- `check_db_ready`
- `get_kb_data`
- `get_files`
- `get_document_text`
- `load_chat_sessions`
- `get_search_history`
- `get_search_document_content`
- `search_documents`

Gate: Electron loads real persisted data with no preview/simulation paths.

### Group B — Database mutations

- `save_chat_session`
- `delete_chat_session`
- `save_file_record`
- `delete_file_record`
- `delete_file_nodes`
- `save_search_document_result`
- `mark_search_document_ingested`
- `mark_search_knowledge_ingested`

Gate: mutations made by either host are visible to the other after reload/reopen,
with only one write-capable host active at a time.

### Group C — Memory and graph work

- `enrich_prompt`
- `process_message`
- `process_turn`
- `ingest_text`
- `run_consolidation`
- `analyze_and_merge_kb`

Gate: entity, fact, relation, source provenance, merge, and deletion results are
identical on a fixed input corpus.

### Group D — Files, tools, and cancellation

- `run_quick_extract`
- `run_ocr`
- `run_search`
- `cancel_process`

Gate: bundled helpers resolve correctly, progress arrives, cancellation reaps
the subprocess, and no zombies remain.

### Group E — Binary and streaming

- `generate_tts`
- `stop_tts`
- `stream_chat`

Gate: WAV bytes arrive as binary data, playback/lip sync works, chat deltas are
ordered, cancellation closes the Rust/Ollama operation, and long streams do not
leak listeners.

## 9. Events, Progress, and Cancellation

Preserve the current event names and payload fields. Add an `operationId`
alongside existing fields so concurrent work cannot update the wrong panel.

Recommended event rules:

- command request ID becomes the default operation ID;
- returned OCR/search PID remains available for the existing UI;
- Electron subscribes once in main and forwards only allowlisted events;
- renderer listener removal must stop forwarding to destroyed windows;
- completion and failure must close request-scoped streams exactly once;
- `cancel_process` stays idempotent;
- Electron shutdown cancels/reaps OCR and Meta Search children before killing the
  Rust sidecar.

## 10. External Services

### Keep as external local services

- Ollama
- SurrealDB
- Qdrant
- SearXNG
- Lightpanda

Do not embed Docker or Ollama into Electron. The desktop application should:

- perform concurrent startup health checks;
- show precise degraded/offline states;
- keep UI and Live2D rendering functional while services are unavailable;
- retry connections with bounded backoff;
- provide a copyable diagnostic report;
- offer documented `docker compose up -d`, not silently start privileged services.

### Improve backend readiness

The current `db_ready` value is largely determined at startup. Add a backend
status model that can reconnect after services start:

```text
BackendStatus
├─ sidecar
├─ surrealdb
├─ qdrant
├─ ollama
├─ searxng
├─ lightpanda
├─ tts
└─ helper binaries
```

Expose the status through `alice:backend-status` and a Rust command/event.
Database recovery should not require restarting Electron.

## 11. Packaging

### Electron resources

Package:

```text
resources/
├── bin/alice-backend
├── tools/ocr-helper
├── tools/meta_search
└── tts/tts_piper_amy_low/
```

The Live2D model remains in the Vite `dist/` payload.

Configure `electron-builder` with `extraResources` for binaries and the TTS model.
The backend and helper binaries must retain executable permissions in AppImage
and Debian packages.

### Build pipeline

Add scripts equivalent to:

```text
backend:build          cargo build --release -p alice-sidecar
electron:stage         build frontend + copy sidecar/helpers/resources
electron:pack          unpacked application
electron:build         AppImage + deb
electron:check         Rust tests + JS tests + build + package inspection
```

The staging step must fail if a required binary or TTS model is absent. It must
print the exact missing dependency and build/download command.

### Native dependency checks

Before publishing:

- inspect `ldd` for `alice-backend`, OCR, and Meta Search;
- test Sherpa-ONNX loading from packaged resources;
- test paths containing spaces and non-ASCII characters;
- verify execution inside AppImage extraction/mount paths;
- verify the Debian package on a clean supported distribution.

## 12. Data Compatibility and Concurrency

No SurrealDB table, Qdrant collection, vector dimension, record key, or graph
serialization format should change merely because the host changes.

Before mutation tests:

- back up the SurrealDB and Qdrant data directories;
- capture counts and representative records;
- ingest a fixed document through Tauri;
- read and modify it through Electron;
- reopen Tauri and verify the same result.

Running Tauri and Electron simultaneously against the same in-memory graph can
cause last-writer-wins graph synchronization. Add a shared backend lock keyed by
namespace/database, preferably under the OS runtime directory. The second host
must clearly refuse write-capable backend startup or enter an explicitly
read-only diagnostic mode.

## 13. Security Requirements

- Keep `contextIsolation: true`, `sandbox: true`, and `nodeIntegration: false`.
- Keep the strict CSP and CSP-safe Pixi shader adapter.
- Never expose a generic shell command or unrestricted child-process API.
- Spawn the Rust sidecar and helpers without `shell: true`.
- Canonicalize and validate frontend-supplied file paths in Rust.
- Preserve current source-file and text-payload limits.
- Cap transport frames and outstanding requests.
- Validate command names in Electron and Rust.
- Keep secrets and SurrealDB credentials outside renderer memory.
- Redact credentials, document bodies, and full prompts from production logs.
- Treat sidecar protocol errors as fatal and restart only within the bounded
  policy.
- Sign/hash packaged sidecar resources when release signing is introduced.

## 14. Test Plan and Release Gates

### Rust tests

- `alice-core` builds and tests without Tauri.
- Every dispatcher method accepts the current camelCase frontend parameters.
- Every result/error serializes through MessagePack.
- EventSink and StreamSink preserve ordering.
- invalid commands, malformed parameters, and oversized payloads are rejected.
- cancellation kills and reaps helper processes.
- resource resolution covers development and packaged layouts.

### Protocol tests

- fragmented header and payload reads;
- several frames in one read;
- 50 MiB text frame;
- binary WAV response;
- concurrent out-of-order responses;
- stream plus global event traffic;
- child crash with pending requests;
- version mismatch;
- malformed/oversized frame;
- graceful and forced shutdown.

### Electron tests

- trusted and untrusted renderer IPC calls;
- preload exposes no Node.js primitives;
- native dialog/open-path behavior;
- backend unavailable, delayed, ready, degraded, crash, and restart states;
- pending request cleanup when a window closes;
- AppImage/deb resource discovery.

### Feature parity matrix

Run the same scenarios in Tauri and Electron:

1. create, reload, rename, and delete chat sessions;
2. stream a chat reply, schedule TTS, persist the turn, and retrieve memory;
3. Quick Extract TXT, Markdown, JSON, DOCX, and PDF;
4. OCR a PDF and cancel an OCR run;
5. approve a document and verify chunks, nodes, facts, relations, and provenance;
6. delete the file and verify vectors and source-owned graph data are removed;
7. run knowledge-document and scientific-paper searches;
8. persist search history, extracted text, OCR text, and ingestion state;
9. run AI graph clean/merge and verify edge rewiring;
10. restart the app and verify all persistent state.

### Performance gate

On the existing 3440×1440 display:

- fullscreen quality mode;
- Live2D active;
- status, chat, OCR, search, and KB panels open;
- median renderer rate at 60 FPS;
- p95 frame time at or below 20 ms during idle animation;
- no sustained frame collapse during chat streaming;
- backend work must not block the Electron main or renderer threads;
- no increasing listener/process/memory count during a 30-minute soak.

### Shutdown gate

After closing Electron:

- no `alice-backend` process remains;
- no OCR or Meta Search helper remains;
- no incomplete protocol frame or unhandled rejection is logged;
- database writes already acknowledged remain durable.

## 15. Phased Delivery

### Phase 0 — Freeze contracts and baselines

- Record all command names, arguments, results, events, and current errors.
- Add a parity checklist and a fixed test corpus.
- Save Tauri functionality and performance baselines.

Exit gate: contract inventory covers every registered command and frontend call.

### Phase 1 — Extract the Tauri-free core

- Introduce `BackendState`, `BackendApi`, `BackendError`, `EventSink`,
  `StreamSink`, and `ResourceLocator`.
- Move or adapt db, memory, graph, process, tool, and TTS modules.
- Convert Tauri commands to thin adapters.

Exit gate: Tauri passes existing tests and manual workflows through `alice-core`.

### Phase 2 — Implement the Rust sidecar

- Add framed MessagePack protocol and handshake.
- Add the command dispatcher, streams, events, cancellation, and shutdown.
- Add protocol/unit tests and a CLI diagnostic mode.

Exit gate: a Node test client can execute every command category without Electron.

### Phase 3 — Connect Electron main and preload

- Add `SidecarManager`.
- Add Electron IPC handlers and backend status.
- Implement lifecycle, backpressure, timeouts, crash handling, and event routing.

Exit gate: Electron reads real KB/files/sessions and performs basic mutations.

### Phase 4 — Remove frontend Tauri coupling

- Add `invokeStream`.
- Remove direct Tauri imports/checks from UI modules.
- Enable real Electron OCR/search/ingestion/TTS/chat paths.
- Remove Electron simulation wording and behavior.

Exit gate: every UI feature uses the runtime-neutral adapter.

### Phase 5 — Package all Rust resources

- Build and stage `alice-backend`, helper binaries, and TTS resources.
- Configure `electron-builder`.
- Add development and packaged resource resolution.

Exit gate: AppImage and Debian packages work without the repository checkout.

### Phase 6 — Service resilience and diagnostics

- Add reconnectable backend/service status.
- Improve offline/degraded UI and diagnostic export.
- Add shared data-writer lock.

Exit gate: services can be started after Electron and recover without app restart.

### Phase 7 — Parity, performance, and soak testing

- Complete automated and manual parity matrix.
- Fix sidecar lifecycle, listener, cancellation, and packaging defects.
- Confirm the 3440×1440 performance gate.

Exit gate: zero known feature-parity failures and no orphan processes.

### Phase 8 — Production cutover and Tauri removal

- Update documentation and release workflow.
- Tag the final verified Tauri reference for rollback through Git history.
- Make Electron + `alice-backend` the production desktop application.
- Remove `src-tauri`, Tauri dependencies/plugins, conditional Tauri frontend
  branches, and the Tauri build pipeline.
- Run the full Electron production matrix again after removal.

Exit gate: the shipped application contains Electron and the Rust backend only;
the last working Tauri version remains recoverable from its Git tag.

## 16. Recommended First Implementation Slice

The first code slice should be small enough to prove the architecture without
moving the most complex workflows:

1. create `alice-core` with `BackendState`, `BackendApi`, `BackendError`,
   `EventSink`, and `ResourceLocator`;
2. migrate `check_db_ready`, `get_kb_data`, `get_files`, and
   `get_document_text`;
3. make Tauri wrappers call those core operations;
4. create `alice-sidecar` with handshake and framed MessagePack;
5. implement Electron `SidecarManager`;
6. make Electron load real KB/file data;
7. add contract, protocol-fragmentation, crash, and shutdown tests.

This slice proves state ownership, transport, packaging direction, and dual-host
compatibility before streaming, TTS, OCR, search, or large ingestion operations
are moved.

## 17. Definition of Done

The migration is complete only when:

- Electron reports the Rust backend ready in development and packaged builds;
- all 29 commands and all existing events/streams work through Electron;
- no Electron UI feature uses preview/simulation because of missing Rust IPC;
- the same persistent data opens correctly in either host;
- OCR, Meta Search, and TTS resources work outside the repository;
- cancellation and application shutdown leave no orphan processes;
- service recovery works without restarting the desktop application;
- Electron passes the full feature parity matrix;
- Electron passes the fullscreen 3440×1440 performance and soak gates;
- Tauri passed the reference parity gate before its tagged code was removed;
- the final shipped and maintained architecture is Electron + Rust only.
