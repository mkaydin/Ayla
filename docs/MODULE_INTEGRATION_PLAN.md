> Historical design record. Runtime and setup instructions are superseded by [README](../README.md) and the [production readiness plan](PRODUCTION_READINESS_PLAN.md).

# Ayla Speech, OCR, and Code Intelligence Integration Plan

Status: implementation in progress  
Prepared: 2026-08-17  
Primary desktop target: Electron/Chromium  
Compatibility target: Tauri 2

Implementation checkpoint (2026-08-17):

- Phase 0 provenance/model licensing is established for the imported speech
  assets; the OCR prototype license question remains open.
- Phase 1 runtime capabilities and verified model-manifest foundations are in
  place for speech. A filesystem-backed artifact foundation now stores OCR and
  code-analysis outputs atomically and exposes fixed-ID, 2 MiB-bounded preview
  reads. Persisted run indexing and the global inference coordinator remain.
- Phase 2 Supertonic TTS is implemented inside Ayla with Piper rollback, lazy
  loading, native cancellation, selectable Turkish/English generation,
  Turkish normalization, both-host routing, Status-panel persistence, and
  package verification.
- Phase 3 CPU Omnilingual STT and Silero VAD are implemented inside Ayla. The
  Chat workflow now records between two microphone presses and only submits the
  bounded recording after capture stops. Turkish and English are recognized in
  automatic-language mode. A persistent Status-panel System STT switch is kept
  separate from manual Chat dictation so it can arm future system commands
  without taking away explicit voice input. The chosen capture contract is
  renderer `getUserMedia` plus bounded 16 kHz PCM16
  commands, avoiding a new native audio/system-library dependency. Dictation
  previews partials and inserts finals without auto-send.
- The Phase 4 local GLM OCR path is implemented in-place in `ayla-ocr`:
  PDFium supplies rendering and text geometry, page recognition is bounded and
  cancellable, output passes repetition/prompt-echo/size gates, merge order is
  deterministic, and Markdown plus layout JSON are stored as Ayla artifacts.
  The current panel shows stages, model, and warnings while retaining the old
  response fields. A generated-PDF sidecar diagnostic passed against
  `glm-ocr:latest` with no Poppler process or temporary render leak. The larger
  golden PDF matrix now covers scanned/raster-only, two-column, table,
  equation, figure/caption, and no-text-layer pages.
- Phase 5 now creates a separate KB-ready OCR
  document and structured graph preview for every successful run. It assigns
  content-stable document IDs, page/heading/figure entities, page citations,
  containment relations, and preserves full OCR evidence. The UI previews all
  four artifacts, ingests the full KB artifact server-side, and independently
  approves/removes graph-v2 provenance. A content-and-options-keyed private
  per-page cache commits successful OCR immediately and restores it on retry.
  Explicit evidence-only, deterministic, and semantic modes are available.
  Semantic mode uses only a loopback Ollama model, extracts cited summaries,
  concepts, facts, and cross-page relationships in bounded chunks, and retains
  deterministic output with a warning if enrichment fails.
- Phase 7 code-analyzer completion is implemented. The analyzer source is
  imported as `ayla-code-analysis`, returns
  structured entities/relations, uses content-based repository identity, and
  defaults semantics and remote Git off. Deterministic analysis now has
  file/byte limits, cooperative cancellation, progress events, sanitized local
  artifacts, Electron/Tauri commands, and a runtime-neutral directory/snippet
  preview panel. Atomic `run.json` records provide durable history and reopening;
  parent-linked re-analysis produces logical-identity diffs; approved revisions
  replace approved parent provenance in one staged graph commit. Approval also
  embeds entities and bounded documentation/KB chunks into the dedicated
  Qdrant `code_analysis` collection. Optional semantics permit only a loopback
  Ollama endpoint, with explicit consent required for cloud-tagged models.
  Remote Git remains rejected. Full-run
  deletion removes graph provenance, run-scoped vectors, artifacts, history,
  and private snippet snapshots. Snippet snapshots are mode `0600`, have no
  public artifact ID, and are used only to reproduce an explicit re-analysis.
- The Phase 6 graph-v2 store converts analyzer output losslessly into validated
  UID-keyed records, rejects missing endpoints/oversized records, supports
  deterministic legacy IDs, merges re-analysis provenance, stages every full
  snapshot, backs up the previous v2 tables, commits transactionally, and can
  roll back by backup or source. The code panel requires explicit approval
  before writing these isolated v2 tables; it never writes the legacy graph.
- The first Phase 6 application migration is complete: the Knowledge Base
  panel dual-reads legacy and bounded Graph-v2 projections, filters structured
  entities and relations by label/origin/source/text, and exposes a persisted
  audit history for imports, removals, and rollbacks. Restores are recoverable
  because the current graph is backed up before applying the selected backup.
- Phase 6 is now complete at the application layer. Ayla performs an automatic,
  idempotent initial legacy backfill, continuously refreshes only deterministic
  `legacy:` UIDs after legacy graph writes, and preserves native OCR/code UID
  namespaces. The UID-keyed Graph-v2 canvas is the default for profiles without
  an explicit choice, while the legacy canvas remains a persisted rollback
  selection for one compatibility release. Structured entities, relations,
  and audit history support bounded byte-safe pagination. Recovery tables keep
  the newest 25 backups automatically, support explicit maintenance, and mark
  expired backups without deleting audit history.
- Phase 5 OCR advanced processing is implemented, including cache/performance
  metrics, a 5,000-page deterministic assembly bound test, and UI/runtime
  configuration. Phase 7's application completion is implemented; broader
  inference coordination and a future explicitly enabled remote-Git policy
  remain release-hardening work.

Current verification after OCR advanced processing:

- JavaScript: 18 test files passed;
- Rust workspace: 126 tests executed, 123 passed and 3 ignored;
- Electron and Tauri adapters compile against the additive OCR and analyzer
  command surfaces;
- live Supertonic validation generated both Turkish and English F1 WAV output;
- code analyzer sidecar fixture: 5 entities/4 relations, all progress stages,
  sanitized artifacts, durable reopen, private `0600` snippet snapshot replay,
  preview-API denial, one-method logical revision diff, snapshot deletion, and
  the offline approval contract passed;
- isolated live analyzer fixture: entity/document vectors were written to and
  deleted from Qdrant, graph-v2 approval/removal/rollback passed, full-run
  deletion removed every owned layer, and an approved 4-entity revision was
  atomically replaced by its 5-entity child without retaining parent provenance;
- isolated live graph-v2 fixture: 4 nodes/3 edges committed transactionally;
  bounded application reads exposed import/removal history, rollback restored
  the removed source, pagination returned stable offsets, legacy backfill
  completed, retention kept 3 requested backups while expiring older audit
  targets, and final cleanup returned the graph to 0 nodes/0 edges;
- live OCR fixture: faithful Markdown, layout JSON, KB-ready Markdown, and
  structured JSON persisted; 3 OCR knowledge entities/2 relations committed
  and source removal returned the isolated graph to 0 nodes/0 edges;
- live generated fixture: 1/1 page recognized by local GLM OCR, caption
  geometry found, two artifacts persisted, and zero temporary outputs leaked.

## 1. Executive decision

Integrate all four modules behind Ayla's existing host-neutral Rust core and its
single private MessagePack sidecar. Do not ship the Python test applications,
add sibling-repository path dependencies, or introduce one helper process per
feature.

The recommended end state is:

- `ayla-stt`: a new Rust crate containing Omnilingual ASR, Silero VAD, device
  capture/session control, and optional transcript repair;
- `ayla-tts`: a new Rust crate containing the multilingual Supertonic F1 engine
  with Turkish/English selection and Turkish text normalization, with
  `ayla-core` retaining the application service facade;
- `ayla-ocr`: an in-place upgrade of the existing crate using the GLM OCR,
  layout, assembly, and validation work from `glm_ocr_tests`;
- `ayla-code-analysis`: a linked Rust library imported from
  `kb_ready_code_doc`, extended to return structured nodes and edges rather
  than only rendered Markdown;
- `ayla-core`: the only application orchestration layer, responsible for
  configuration, cancellation, progress, persistence, inference scheduling,
  privacy policy, and resource discovery;
- `ayla-sidecar`: the only Electron child process. It continues to expose
  versioned commands, events, streams, structured errors, cancellation, and
  binary audio;
- the renderer: a runtime-neutral UI that never launches tools, reads arbitrary
  paths, calls Ollama directly, or owns model/runtime policy.

This preserves the architecture already enforced by Ayla's tests and packaging
scripts while avoiding four new deployment and supervision surfaces.

## 2. Evidence and current baseline

### 2.1 Ayla

Ayla currently has:

- a vanilla JavaScript/Vite renderer shared by Electron and Tauri;
- a host-neutral `ayla-core` Rust crate;
- one `ayla-sidecar` executable using length-prefixed MessagePack;
- in-process `ayla-ocr` and `ayla-research` libraries;
- a sherpa-onnx 1.13.4 Rust binding already linked into the core;
- Piper `amy-low` TTS returning WAV bytes to the Web Audio player;
- cancellable OCR/search and request-scoped events;
- SurrealDB, Qdrant, and a provenance-aware but name-keyed knowledge graph;
- Electron resource staging and package inspection;
- an Electron renderer with Node integration disabled, context isolation and
  sandboxing enabled, and a narrow preload bridge.

Baseline verification on 2026-08-17:

- JavaScript: 16 test files passed;
- Rust workspace: 69 tests executed, 66 passed and 3 ignored;
- the worktree had no reported tracked modifications before this plan was
  added.

### 2.2 Source modules inspected

| Module | Inspected revision/state | Tests | Main assets and dependencies | Integration readiness |
| :--- | :--- | :--- | :--- | :--- |
| STT, `ayla_stt_test_module` | `f58e86d` | 8 passed | Python 3.11, sherpa-onnx 1.13.4, sounddevice, Omnilingual 300M, Silero VAD | Algorithmically ready; application lifecycle and packaging are not implemented |
| TTS, `ayla_tts_test_module` | `6139e55`; untracked demo MP4 exists | 2 passed | Python 3.11, sherpa-onnx 1.13.4, Supertonic 3 INT8 F1 | Engine is small and stable; cancellation, status, and Ayla packaging remain |
| OCR, `glm_ocr_tests` | `b28f605` | 33 passed | Rust 2024, Poppler, GLM OCR, Nemotron assembly, reqwest 0.13 | Feature-rich and tested; must become a library and shed platform/toolchain conflicts |
| Code analyzer, `kb_ready_code_doc` | `7452645` | 9 passed | Rust library/CLI, Tree-sitter, optional Ollama, Git | Best library boundary of the four; needs progress/cancellation, structured export, and Ayla persistence |

The source revisions above should be recorded in an import provenance file.
The untracked TTS demo video must not be imported accidentally.

## 3. Important findings that shape the design

1. Ayla's vendored sherpa-onnx Rust API already exposes the exact primitives
   needed by the Python experiments: Omnilingual offline ASR, Silero VAD,
   Supertonic TTS, generation callbacks, and provider selection. Porting the
   small Python orchestration layers to Rust is substantially safer than
   packaging Python and a second native runtime.

2. Ayla's prebuilt sherpa libraries are CPU builds. A `provider=cuda` setting
   alone does not create CUDA support. The first production STT release should
   use the 350 MB Omnilingual INT8 CPU model. CUDA should be a separate,
   capability-detected build/profile after a native-library packaging spike.

3. The STT experiment's repair default is `gemma4:cloud`, so transcript text
   can leave the machine. Ayla must not enable that silently. Repair must have
   explicit `off`, `local`, and `cloud` modes, with `off` or a verified local
   model as the default.

4. The TTS experiment labels sample files with emotions, but its public engine
   only sets voice F1, language, speed, and generation steps. Do not expose an
   emotion control until a real model parameter and quality test support it.

5. The GLM OCR prototype depends on Poppler commands and Rust edition 2024,
   while Ayla promises Rust 1.77.2 and currently bundles PDFium. Importing it
   unchanged would regress portability and toolchain compatibility.

6. PDFium already exposes page text characters and bounds. The GLM layout
   algorithms should consume PDFium geometry and reuse the existing PDFium
   renderer. This removes the Poppler runtime dependency and keeps OCR inside
   the single sidecar.

7. The GLM prototype currently fails on PDFs without a text layer because it
   requires geometry. Ayla should still OCR such documents: mark layout as
   unknown/single-column and omit geometry hints rather than rejecting them.

8. `kb_ready_code_doc` produces rich, stable UIDs, labels, properties,
   provenance, and edge metadata, but `GeneratedDocuments` exposes only two
   Markdown strings and diagnostics. Its internal knowledge package must be
   made public/serializable for lossless ingestion.

9. Ayla's current graph is keyed by lowercase display name and stores only six
   coarse node types. Directly squeezing repository symbols into it would
   collapse same-named symbols across projects and discard code analyzer
   metadata. A backward-compatible graph schema v2 is a prerequisite for full
   code analyzer integration.

10. Supertonic's local model directory contains an MIT license file, while its
    README says the model is OpenRAIL-M. Redistribution must wait until the
    authoritative model license and notice obligations are resolved. The
    Omnilingual model files inspected are Apache-2.0. The OCR crate has no
    declared package license and needs an explicit import/license decision.

## 4. Target architecture

```text
Renderer (no Node, no direct Ollama/filesystem/process access)
  |
  | invoke / cancel / events / streams / binary WAV
  v
Electron preload + main process
  |
  | framed MessagePack over private stdin/stdout
  v
ayla-sidecar (the only packaged executable)
  |
  v
ayla-core
  |-- SpeechCoordinator
  |     |-- ayla-stt -> cpal input -> resampler -> Silero VAD -> Omnilingual
  |     `-- ayla-tts -> Supertonic F1 -> PCM/WAV
  |-- InferenceCoordinator -> bounded Ollama workloads
  |     |-- chat / memory extraction
  |     |-- GLM OCR page recognition
  |     |-- optional OCR document assembly
  |     `-- optional code semantic analysis / transcript repair
  |-- ayla-ocr -> PDFium render + geometry -> GLM -> document artifacts
  |-- ayla-code-analysis -> discovery + Tree-sitter + optional semantics
  `-- Persistence
        |-- SurrealDB: runs, artifacts, structured graph/provenance
        `-- Qdrant: document chunks and code/entity embeddings
```

The Tauri host calls the same core services through thin adapters. New
business logic must not be implemented in Electron main, preload, or Tauri
commands.

## 5. Repository layout after integration

```text
crates/
  ayla-core/
    src/
      speech.rs                 # coordination and half-duplex policy
      inference.rs              # Ollama workload limits
      artifacts.rs              # artifact/run persistence facade
      graph_import.rs           # structured imports and rollback
      stt.rs                    # core service facade
      tts.rs                    # core service facade; WAV/application contract
      code_analysis.rs          # orchestration and persistence
  ayla-stt/
    src/{engine,vad,capture,repair,models,types}.rs
  ayla-tts/
    src/{engine,text,models,types}.rs
  ayla-ocr/
    src/{lib,render,geometry,ocr,assemble,output,types}.rs
  ayla-code-analysis/
    src/...                     # imported/adapted kb_ready_code_doc library
  ayla-sidecar/
src/ui/
  voice-input.js
  code-analysis-panel.js
styles/
  voice.css
  code-analysis.css
docs/
  module-provenance.md
  model-licenses.md
```

Names may be consolidated later, but separate inference crates make dependency,
unit-test, and host-neutrality boundaries explicit.

## 6. Shared foundations to build first

### 6.1 Import and dependency policy

- Import source into this repository; do not reference `/home/mka/Code/...` in
  Cargo manifests, runtime defaults, tests, or documentation.
- Record source repository, revision, imported paths, local modifications, and
  license in `docs/module-provenance.md`.
- Keep CLI frontends only as examples or diagnostics. Ayla must call library
  APIs directly.
- Normalize Rust edition and minimum toolchain. Either:
  - preserve Ayla's Rust 1.77.2 promise by adapting the OCR crate to edition
    2021 and compatible dependency versions; or
  - deliberately raise the whole workspace MSRV and update README, CI, doctor,
    and packaging together.
- Prefer the first option unless a concrete dependency makes it impossible.
- Normalize `reqwest` to Ayla's 0.12 line and reuse its TLS policy; do not carry
  both reqwest 0.12 and 0.13 merely because the prototype used 0.13.
- Reuse the one sherpa-onnx dependency and one Tokio runtime.

### 6.2 Application configuration

Split model/runtime settings from `MemoryConfig` into a composed `AppConfig` or
add focused sub-configs:

```text
SpeechConfig
  stt_model_dir, stt_provider, stt_threads, input_device
  vad_threshold, min_silence_ms, max_utterance_secs
  transcript_repair_mode, transcript_repair_model
  tts_engine, tts_model_dir, tts_threads, tts_steps, tts_speed

OcrConfig
  ocr_model, ocr_parallelism, ocr_num_ctx, dpi, max_dimension
  assembly_mode, assembly_model, assembly_num_ctx

CodeAnalysisConfig
  semantic_model, allow_semantic_cloud, allow_remote_git, max_files, max_total_bytes
  semantic_symbol_limit, semantic_document_limit
```

Environment keys should use `AYLA_` consistently. Proposed keys include:

- `AYLA_STT_MODEL_DIR`, `AYLA_STT_THREADS`, `AYLA_STT_DEVICE`;
- `AYLA_STT_REPAIR_MODE`, `AYLA_STT_REPAIR_MODEL`;
- `AYLA_TTS_ENGINE`, `AYLA_TTS_LANGUAGE`, `AYLA_TTS_MODEL_DIR`,
  `AYLA_TTS_THREADS`, `AYLA_TTS_STEPS`, `AYLA_TTS_SPEED`;
- `AYLA_OCR_MODEL`, `AYLA_OCR_PARALLEL`, `AYLA_OCR_NUM_CTX`,
  `AYLA_OCR_ASSEMBLY_MODE`, `AYLA_OCR_ASSEMBLY_MODEL`,
  `AYLA_OCR_ASSEMBLY_ALLOW_CLOUD`;
- `AYLA_CODE_ANALYSIS_MODEL`, `AYLA_CODE_ANALYSIS_ALLOW_CLOUD`,
  `AYLA_CODE_ANALYSIS_ALLOW_GIT`.

Validate ranges once in Rust and return effective settings through a read-only
capabilities/status command.

### 6.3 Runtime capabilities and model management

Add `get_runtime_capabilities`, returning availability and reasons rather than
having the Status panel assume TTS is online:

```json
{
  "stt": {"available": true, "provider": "cpu", "model": "omni-300m-int8"},
  "tts": {"available": true, "engine": "supertonic", "voice": "F1"},
  "ocr": {"available": false, "reason": "glm-ocr model not installed"},
  "codeAnalysis": {"available": true, "gitAvailable": true}
}
```

Create a model manifest containing logical ID, version, files, byte sizes,
SHA-256 hashes, license reference, supported providers, and download URL.

- Store downloaded optional models below Ayla's application data directory,
  not in the source tree.
- Download to a temporary name, verify every hash, then atomically rename.
- Support resume if the HTTP source permits it.
- Never execute downloaded content.
- Allow a fully offline/manual installation by accepting a verified directory.
- Keep the CPU Omnilingual INT8 model as the default STT choice.
- Decide separately whether Supertonic is bundled or first-run downloaded after
  its license gate is closed.
- Add package flavors only if needed: `base` and `offline-speech`. Do not make
  the ordinary package silently grow by more than 500 MB.

### 6.4 Cancellation and workload control

Use cancellation tokens owned by services, not only aborted request futures.
This is essential because `spawn_blocking` and native inference continue after
an async request is aborted unless explicitly told to stop.

Add an `InferenceCoordinator` with separately bounded classes:

- interactive: chat and transcript repair;
- speech CPU: STT and TTS;
- vision: OCR page requests;
- batch semantic: OCR assembly and code analysis.

Interactive work receives priority. Parallel OCR must be configurable and
capped by the coordinator rather than trusting `OLLAMA_NUM_PARALLEL` blindly.
Cancellation must abort/join all spawned page tasks and clean temporary files.

### 6.5 Operation and artifact model

Long tasks should share a durable shape:

```text
Operation
  id, kind, state, created_at, updated_at, progress
  source, options, error, retryable, artifact_ids

Artifact
  id, operation_id, kind, media_type, path, byte_size, sha256
  source_name, created_at, metadata
```

Store large Markdown/layout outputs as application-data artifacts and persist
metadata in SurrealDB. Return summaries and IDs over IPC; provide bounded reads
for previews. This avoids relying on the 64 MB protocol frame for arbitrarily
large PDFs or repositories.

Artifacts remain previews until the user approves them. Approval is the single
commit point for Qdrant and graph ingestion, matching Ayla's existing document
lifecycle.

## 7. STT integration

### 7.1 Porting scope

Port and preserve:

- the Omnilingual INT8/FP32 model catalog and provider defaults;
- 16 kHz mono decoding;
- Silero VAD thresholds, minimum speech, minimum silence, and 30-second maximum
  utterance;
- bounded partial decoding and finalized utterances;
- Turkish/English script validation;
- bounded, non-blocking optional transcript repair;
- raw versus repaired transcript visibility.

Do not port:

- the Python CLI as the production runtime;
- `sounddevice`, NumPy, soundfile, Python virtual environments, or CUDA wheels;
- cloud repair as the default.

### 7.2 Audio capture and processing

Use `cpal` in `ayla-stt` for the first native implementation.

1. Enumerate input devices and stable-enough UI identifiers.
2. Open the selected device at a supported native rate and channel count.
3. Keep the real-time callback minimal: down-mix/copy into a bounded channel;
   never run VAD, allocation-heavy concatenation, or ASR in the callback.
4. Resample to 16 kHz mono on a dedicated worker.
5. Feed fixed windows to Silero VAD.
6. Decode partials no more frequently than the configured interval.
7. Decode each finalized VAD segment once and emit ordered events.
8. On stop, flush VAD, emit any final segment, stop the stream, join workers,
   and release the microphone.

Before committing to `cpal`, add a packaged Linux spike that verifies ALSA and
PipeWire behavior. If cross-platform package results are unacceptable, retain
the same service/event contract and move capture to `getUserMedia`, sending
bounded PCM16 chunks through explicit `stt_push_audio` commands. Do not mix the
two approaches in one release.

### 7.3 Service and protocol contract

Recommended commands:

| Command | Purpose |
| :--- | :--- |
| `get_stt_status` | Effective model/provider/device/session state |
| `list_stt_devices` | Enumerate usable input devices |
| `start_stt` | Start one capture session; return `sessionId` immediately |
| `stop_stt` | Flush and stop the named session |
| `set_stt_device` | Change the idle/default device |

Recommended request-scoped/global events:

| Event | Required payload |
| :--- | :--- |
| `stt-state` | `sessionId`, `state`, `device`, `model`, `provider` |
| `stt-level` | `sessionId`, throttled RMS/peak value |
| `stt-partial` | `sessionId`, `sequence`, `text` |
| `stt-final` | `sessionId`, `sequence`, `rawText`, `text`, `repairStatus` |
| `stt-error` | `sessionId`, structured code/message/retryable flag |

Add the methods/events to the Rust catalogs, sidecar dispatcher, Electron event
forwarder, Tauri adapters, protocol contract tests, and renderer IPC tests. The
MessagePack envelope can remain protocol version 1 because no framing change is
needed with native capture.

### 7.4 User experience and safety

- Add a microphone button beside the chat input plus device and language/repair
  settings in Status.
- Start with push-to-talk and tap-to-dictate modes. Continuous listening can
  follow after lifecycle testing.
- Final transcripts populate the composer; do not auto-send by default. This
  prevents a recognition error from executing an unintended request.
- Show partial text visually but replace it with the final text atomically.
- Let users choose raw or repaired text when repair changed the utterance.
- Clearly mark cloud repair and require explicit opt-in before any transcript
  leaves the device.
- Use half-duplex speech by default: starting STT stops/pauses TTS; active TTS
  suppresses microphone recognition. Add headset/full-duplex mode only after
  echo tests.
- Reflect listening/speaking states in the animation coordinator without
  letting them override higher-priority service-loss/error states.

### 7.5 STT acceptance gates

- deterministic WAV-fixture tests for raw final transcripts;
- Turkish, English, and code-switched golden corpus with WER/CER reporting;
- partials are ordered and never emitted after final for the same sequence;
- silence, very short noise, device removal, buffer overflow, and 30-second
  utterances behave predictably;
- finalization occurs within one second of the configured silence endpoint on
  the reference machine;
- CPU real-time factor remains below 1 on the minimum supported machine;
- start/stop can run 100 cycles without leaked devices, tasks, or memory growth;
- packaged Electron can capture audio on Linux; Windows/macOS are release gates
  when those packages are claimed;
- cloud repair is proven off by default in an integration test.

## 8. TTS integration

### 8.1 Engine migration

Move the Supertonic engine into `ayla-tts` using Ayla's existing Rust binding:

- configure `OfflineTtsSupertonicModelConfig` with the seven required files;
- use CPU provider, F1/SID 0, selectable `lang=tr|en`, default 8 steps, speed
  1.0, and validated thread count;
- port whitespace collapse and Unicode NFKD normalization with the
  `unicode-normalization` crate;
- preserve WAV PCM16 mono output so `src/tts-player.js` does not need a new
  transport;
- load the engine lazily on first use and cache it;
- expose effective sample rate, voice, language, steps, speed, and engine name
  through capabilities.

### 8.2 Compatibility and rollback

Keep `generate_tts { text, language? } -> binary WAV` and `stop_tts` stable;
omitting `language` preserves the configured default.

During rollout:

- support `AYLA_TTS_ENGINE=piper|supertonic`;
- keep Piper as a rollback engine for one release/testing cycle;
- switch the default to Supertonic only after package, cancellation, Turkish
  listening, and license gates pass;
- remove the Piper resource and code in a later cleanup release, not in the same
  change that introduces Supertonic.

### 8.3 Real cancellation

The current `stop_tts` stops renderer playback but cannot stop native generation.
Use sherpa's progress callback with an atomic cancellation token tied to the
active generation ID. `stop_tts` must:

1. invalidate queued renderer segments;
2. signal the current native generation;
3. prevent cancelled audio from entering the player queue;
4. release the engine mutex promptly;
5. remain idempotent.

Include a monotonically increasing generation/epoch in the renderer queue so a
late response from an old request is discarded.

### 8.4 TTS packaging and acceptance gates

- resolve the OpenRAIL-M versus MIT model-license mismatch before bundling;
- stage and inspect all seven model files, the authoritative license, and model
  metadata;
- update Electron and Tauri resources together;
- update `doctor.sh` and a sidecar feature diagnostic to validate model load;
- verify Turkish diacritics, apostrophes, numbers, abbreviations, code terms,
  and mixed Turkish/English samples;
- require RTF below 1 on the minimum supported CPU and track cold-load latency
  separately from warm synthesis;
- verify repeated partial-response segments speak in order and are deduplicated;
- verify cancellation, mute, avatar lip sync, and backend restart;
- do not claim emotional speech from filenames alone.

## 9. GLM OCR integration

### 9.1 Crate refactor

Upgrade `crates/ayla-ocr` rather than adding a second OCR crate.

Import/adapt:

- `types.rs` page layout structures;
- `geometry.rs` line clustering, column/headline/caption detection, and tests;
- `ocr.rs` GLM prompt, downscaling, retry, repetition detection, fence/entity
  cleanup, and tests;
- `assemble.rs` document and KB assembly, retry, UID cleanup, validation, and
  tests;
- batch/resume concepts from `main.rs`, but expose them as library APIs and
  application operations rather than CLI control flow.

Replace:

- `pdftoppm` with Ayla's PDFium renderer;
- `pdftotext -bbox` with PDFium character/segment bounds converted into the
  geometry module's neutral `Word` records;
- direct environment reads with injected `OcrConfig`;
- direct `eprintln!` progress with callbacks/EventSink and structured logs;
- reqwest 0.13 with the shared/injected HTTP policy;
- detached page tasks with cancellation-aware `JoinSet` ownership.

### 9.2 OCR modes

Expose explicit modes so local/privacy and latency are understandable:

| Mode | Pipeline | Default |
| :--- | :--- | :--- |
| Evidence only | PDFium + GLM OCR + deterministic page merge and cited KB document, no graph records | No |
| Deterministic structured | Evidence output + stable page/heading/figure graph | Yes |
| Ollama semantic | Deterministic structured + concepts/facts/summaries/relationships from the configured local or explicitly enabled cloud model | No |

Semantic assembly rejects non-loopback endpoints. Cloud-tagged models require
explicit consent. The evidence and deterministic modes never invoke the assembly model.

### 9.3 Library result

Return structured results internally:

```text
OcrDocument
  source_name
  markdown_artifact
  kb_artifact?
  layout_artifact
  pages_processed, pages_skipped, total_pages
  page_results[] { page, chars, columns, headings, warnings }
  models { ocr, assembly? }
  warnings[]
```

Keep the existing `run_ocr` response fields during migration so the current OCR
and Search panels continue to work. Add artifact IDs and warnings as additive
fields. Once both panels use artifacts, deprecate returning the entire document
in a single response.

### 9.4 Scanned and mixed PDFs

- If PDFium finds useful text geometry, use it for columns/headings/captions.
- If it finds no text, continue OCR with `layoutSource=none`, one-column/unknown
  hints, and no figure geometry.
- For mixed PDFs, determine layout per page rather than rejecting the document.
- Treat geometry as a hint, never as the content source.
- Preserve GLM OCR text, tables, and LaTeX as the evidence-bearing page output.

### 9.5 UI and ingestion

- Rename the existing OCR tab copy from deepseek to GLM OCR.
- Show page-level progress, current stage, skipped-page warnings, and mode/model.
- Preview final Markdown, layout diagnostics, and optional KB draft in separate
  tabs.
- Keep processing and approval separate.
- On initial rollout, approve the final document Markdown through Ayla's existing
  document ingestion pipeline; store the KB draft as an artifact, not as a
  second source that would duplicate facts.
- After graph v2 lands, optionally import validated structured OCR KB nodes and
  relations directly.
- Preserve existing Search-panel batch OCR behavior and request cancellation.

### 9.6 OCR acceptance gates

- golden PDFs for single-column, two-column, table, LaTeX, figures/captions,
  scanned-only, mixed text/scanned, corrupted, encrypted, and very long cases;
- no Poppler executable is required in development or packaged builds;
- page order is deterministic under parallel OCR;
- cancellation stops and joins all page/assembly work and removes temporary
  pages;
- one failed page does not fail a document if useful pages remain;
- repetition/truncation and minimum-quality gates are reported in warnings;
- no assembly or KB call occurs unless its mode was explicitly selected;
- source text, model, page, and operation provenance survive approval;
- package diagnostics run against a generated test PDF rather than copyrighted
  research papers.

## 10. Graph schema v2 and structured ingestion

This is the highest-risk shared migration and should be isolated from the UI
feature work.

### 10.1 Required model

Extend graph records to preserve identity and metadata:

```text
GraphNodeV2
  uid                 # stable merge key
  name                # display name
  label               # Repository, Function, Concept, ...
  node_type           # existing coarse UI category for compatibility
  aliases[]
  meaning
  origins[]
  sources[]
  source_locations[]
  confidence
  properties          # bounded JSON object
  created, updated

GraphEdgeV2
  uid
  from_uid, to_uid
  relation
  origins[]
  sources[]
  source_locations[]
  confidence
  properties
  created, updated
```

Index nodes by UID. Maintain a secondary normalized-name index for legacy chat
and cleanup operations, but never use display name as the only identity.

### 10.2 Migration

- Add versioned SurrealDB records/tables or version fields; do not mutate all
  live rows in place without a backup.
- Convert legacy nodes to deterministic `legacy:<type>:<hash>` UIDs.
- Convert edges after the node mapping is complete.
- Validate node/edge counts and all endpoints.
- Commit with the existing staging-table snapshot pattern.
- Keep a read adapter for v1 data for one release.
- Update frontend graph models to use UID as the selection/layout key.
- Add source filters and label styling without breaking existing six categories.
- Test source deletion when nodes/edges have multiple provenance sources.

### 10.3 Structured import transaction

Add a single import API accepting validated nodes, relations, embeddings, and a
source/run ID. It must stage, validate, and commit graph plus run metadata as one
logical operation. Qdrant writes need compensating cleanup if the database
commit fails.

Deletion by run/source must remove that provenance and only delete a shared
node or edge when no provenance remains.

## 11. Code analyzer integration

### 11.1 Library changes

Import `kb_ready_code_doc` as `ayla-code-analysis` and preserve its deterministic
parsers and tests. Extend its API so generation returns:

```text
CodeAnalysisResult
  project_name, repository_uid, fingerprint
  human_markdown, knowledge_base_markdown
  entities[], relations[], diagnostics[]
  coverage { files, documents, symbols, semantic targets/completed }
```

Make the package/entity model public and serializable while keeping
`source_excerpt` excluded from rendered/persisted metadata unless explicitly
needed during the active semantic call.

Fix repository identity before using UIDs for persistence:

- Git input: fingerprint normalized remote identity plus commit hash;
- local Git directory: remote identity plus HEAD where available;
- non-Git directory: content fingerprint from selected manifests/documents and
  relative paths, not the machine-specific canonical directory path;
- snippet: content, language, and name fingerprint.

Add a progress callback/cancellation token at discovery, deterministic parse,
semantic batches, rendering, and persistence boundaries.

### 11.2 Semantic provider

Implement an Ayla-owned `LlmClient` adapter so code semantics use:

- Ayla's configured Ollama endpoint/model;
- common timeouts, retry, context, and workload coordination;
- explicit local/cloud disclosure;
- cancellation;
- structured logging without source leakage.

Keep deterministic `--no-semantic` behavior fully functional and make it the
safe fallback when the model is unavailable.

### 11.3 Security and limits

- Local directories must come from the native directory picker or another
  explicit user action.
- Extend dialog normalization with a dedicated `directory` option; do not make
  arbitrary renderer strings equivalent to picker authorization.
- Remote Git is disabled unless configured/enabled in UI.
- Allow public HTTPS by default when Git is enabled. Treat SSH/private access as
  a separate opt-in because it can use local credentials.
- Reject `file://`, local-path, and unsupported schemes in the Git URL field.
- Run Git without a shell, set `GIT_TERMINAL_PROMPT=0`, use shallow/no-tag clone,
  set a timeout, and bound clone size where practical.
- Enforce maximum file count, per-file bytes, total read bytes, entities,
  relations, semantic requests, and operation duration.
- Continue respecting `.gitignore` and known build/vendor exclusions.
- Never persist `.env` values, discovered directory source excerpts, or Git
  credentials. Retain the analyzer's existing key-name-only behavior. A pasted
  snippet may be retained only as a private, permission-restricted run snapshot
  for reproducible re-analysis. The raw snapshot must never enter previews,
  graph records, diagnostics, logs, or direct source-document indexing; only
  the normal approved structured entities and generated documentation may be
  embedded. Run deletion must remove the snapshot.

### 11.4 Commands and events

Recommended commands:

| Command | Purpose |
| :--- | :--- |
| `run_code_analysis` | Analyze directory, URL, or snippet and create preview artifacts |
| `get_code_analysis_runs` | List persisted runs and states |
| `get_code_analysis_run` | Reopen one saved result summary |
| `get_artifact_content` | Fetch one fixed-ID, bounded artifact preview |
| `approve_code_analysis` | Transactionally ingest one run |
| `remove_code_analysis` | Remove graph provenance while retaining the run |
| `delete_code_analysis_run` | Remove graph, vectors, artifacts, and history together |

Recommended events:

- `code-analysis-started`;
- `code-analysis-progress` with stage/current/total/message;
- `code-analysis-warning` with a bounded diagnostic;
- `code-analysis-complete`;
- `code-analysis-error`.

Register `run_code_analysis` and approval as long/cancellable operations in the
Electron sidecar manager.

### 11.5 Persistence and retrieval

- Save both Markdown documents as artifacts.
- Save run metadata atomically beside each full structured artifact so history
  remains available even while databases are offline.
- On approval, import structured graph data through graph v2; do not ask Ayla's
  generic extractor to reinterpret the `.kb.md` prose.
- Store `embeddingText` in the dedicated Qdrant `code_analysis` collection,
  with UID, run ID, repository UID, label, source location,
  and project payload.
- Store bounded human-documentation and KB-document chunks in the same
  dedicated collection with an explicit record-kind payload.
- Deduplicate by repository UID plus entity UID and make re-analysis an
  upsert/new revision rather than an accumulating duplicate.
- Allow deletion/rollback by analysis run while preserving facts shared by
  another source.

### 11.6 UI

Create a dedicated Codebase Analyzer panel rather than overloading the PDF
processor:

- source: local directory, public Git URL, or pasted snippet;
- scope: Documents only (default) or All files;
- semantics: Deterministic, Prioritized, or Exhaustive;
- limits and clear cost/privacy disclosure;
- progress and cancel;
- summary, diagnostics, human documentation, and KB/graph preview tabs;
- explicit Approve & Add to Knowledge Base;
- previous runs with refresh, retry, compare revision, and delete;
- jump from an imported graph node to repository-relative source location.

### 11.7 Code analyzer acceptance gates

- preserve the current Rust/Python/JavaScript/TypeScript and metadata tests;
- add fixture repositories with duplicate symbol names, ignored/vendor trees,
  malformed manifests, Compose, licenses, env templates, and secrets;
- deterministic output is byte-stable for the same content and options;
- the same Git commit analyzed from two directories produces the same IDs;
- cancellation cleans temporary clones and performs no partial approval;
- unavailable Ollama yields complete deterministic output plus diagnostics;
- re-analysis updates one repository revision without duplicate graph nodes;
- deletion removes only the chosen run's provenance and vectors;
- package test covers Git absent/present and remote Git disabled/enabled states.

## 12. Renderer, desktop host, and status integration

### 12.1 Shared renderer rules

- New UI modules import only `src/lib/ipc.js` and other runtime-neutral helpers.
- Add every new UI module to `runtime-neutral-ui.test.js`.
- Do not add direct Tauri imports, Electron preload access, Ollama calls, or
  child-process calls.
- Sanitize all Markdown through the existing renderer.
- Treat source paths and diagnostics as untrusted text.

### 12.2 Electron

Update:

- `electron/main.mjs`: forwarded event allowlist and any trusted directory
  picker policy;
- `electron/sidecar-manager.mjs`: long-command set and shutdown behavior;
- `electron/stage-resources.mjs`: speech model manifest/staging policy;
- `electron-builder.yml`: selected model/license resources;
- `electron/inspect-package.mjs`: exact required files, size, executable mode,
  and forbidden Python/helper artifacts;
- preload only if a generic existing IPC operation cannot express the needed
  safe action.

The package inspection must assert that no Python interpreter, `.venv`, source
module output, sample recording, or extra helper executable was included.

### 12.3 Tauri

Add thin command/event adapters and update bundled resources/capabilities.
Microphone entitlements/permissions must be documented per OS. Tauri parity may
follow Electron feature completion, but core/library tests must remain host
neutral from the start.

### 12.4 Status and diagnostics

- Replace the hard-coded Piper status row with dynamic speech capabilities.
- Show STT model/device/provider, TTS engine/voice, OCR model/mode, and code
  analyzer availability.
- Add setup actions only through a safe model manager; never display a green
  service merely because its code exists.
- Extend `doctor.sh` to check model manifests, licenses, disk space, audio
  runtime, Ollama models, and optional Git.

## 13. Implementation phases

### Phase 0 - Freeze contracts and close legal/toolchain gates

Deliverables:

- provenance and license inventory;
- decision on Supertonic redistribution;
- decision on Ayla MSRV;
- CPU-only STT MVP decision recorded;
- `cpal` packaged Linux spike;
- sample/golden fixtures cleared for redistribution;
- architecture decision records for PDFium geometry and graph v2.

Exit gate: no unresolved license or unsupported-toolchain assumption blocks the
chosen package.

### Phase 1 - Shared platform foundation

Deliverables:

- sub-configs and validation;
- runtime capabilities command;
- model manifest/locator/verification;
- cancellation tokens and inference coordinator;
- operation/artifact storage skeleton;
- protocol catalogs and test helpers prepared for new event families.

Exit gate: an offline sidecar reports exact availability/reasons and all new
foundation tests pass without any model installed.

### Phase 2 - Supertonic TTS behind the existing API

Deliverables:

- `ayla-tts` Rust crate;
- lazy multilingual Supertonic engine, Turkish normalization, and explicit
  Turkish/English request selection;
- real generation cancellation and renderer epoch handling;
- Piper rollback switch;
- resource/license staging and package inspection;
- updated status, diagnostics, and golden audio tests.

Exit gate: old TTS contract remains compatible, warm RTF is below 1 on minimum
hardware, cancellation is real, and packaged Electron speaks Turkish and
English.

### Phase 3 - Native STT and chat dictation

Deliverables:

- `ayla-stt` crate and CPU INT8 engine;
- cpal capture/resampling/VAD worker;
- model installation/verification;
- STT commands/events and sidecar/Tauri routes;
- microphone UI, half-duplex policy, raw/repaired review;
- deterministic WAV diagnostics and manual microphone test plan.

Exit gate: dictation is stable, never auto-sends by default, cloud repair is
off by default, and packaged Linux capture passes repeated lifecycle tests.

### Phase 4 - GLM OCR local extraction

Deliverables:

- PDFium geometry adapter;
- GLM page OCR, bounded parallelism, quality gates, progress, cancellation;
- deterministic document merge and artifact output;
- backward-compatible `run_ocr` result;
- updated OCR and Search panels;
- no-text-layer support and golden PDF suite.

Exit gate: no Poppler dependency, current document workflows still work, and
scanned/two-column/table/math fixtures pass.

### Phase 5 - Optional OCR assembly and KB draft

Deliverables:

- [x] explicit evidence-only, deterministic structured, and semantic modes;
- [x] loopback-only Ollama policy and visible local-processing disclosure;
- [x] bounded chunking, normalization, graph validation, artifacts, and warnings;
- [x] resume from completed page OCR without repeating it;
- [x] golden document-shape matrix and large-document performance bounds.

Exit gate: local-only mode never invokes assembly, long documents resume, and
bad assembly cannot overwrite valid page OCR evidence.

### Phase 6 - Graph schema v2

Deliverables:

- UID-keyed nodes/edges with properties/provenance;
- migration and rollback adapters;
- transactional structured import;
- UI graph UID migration and filters;
- source/run deletion tests.

Exit gate: legacy graph counts and behavior are preserved, analyzer-style
duplicate names remain distinct, and mixed provenance deletion is correct.

### Phase 7 - Code analyzer preview and structured approval

Deliverables:

- `ayla-code-analysis` crate and public structured result;
- stable repository fingerprinting;
- progress/cancellation/security limits;
- Ayla semantic provider;
- run/artifact persistence and Qdrant entity collection;
- Codebase Analyzer panel;
- approval, re-analysis, and rollback flows.

Exit gate: deterministic analysis works without Ollama, semantic analysis is
optional, and approved graph/vector data is lossless and removable by run.

### Phase 8 - Release hardening and cleanup

Deliverables:

- Electron AppImage/deb package validation and claimed OS matrix;
- Tauri parity validation;
- resource size/startup/RAM/CPU/GPU benchmarks;
- updated README, doctor, setup, diagnostics, test plan, and privacy docs;
- feature flags defaulted according to acceptance results;
- removal of Piper and compatibility adapters only after the rollback window.

Exit gate: a clean machine can install, diagnose, use, cancel, restart, and
uninstall every enabled module with no sibling repositories present.

## 14. File-level change map

| Area | Expected changes |
| :--- | :--- |
| Workspace | `Cargo.toml`, `Cargo.lock`, new crates, normalized dependencies/MSRV |
| Core ownership | `crates/ayla-core/src/state.rs`, new speech/inference/artifact/code services |
| Config/resources | `config.rs`, `resources.rs`, `.env.example`, model manifest |
| API/contracts | `api.rs`, `commands.rs`, `events.rs`, structured error codes |
| Sidecar | `dispatcher.rs`, `main.rs`, event sinks, method/capability tests |
| Existing TTS | replace engine implementation in `tts.rs` while keeping facade/IPC |
| Existing OCR | refactor `crates/ayla-ocr`, update `tools/ocr.rs`, result types |
| Graph/persistence | graph v2 model, SurrealDB migration/import, Qdrant code collection |
| Electron | main event forwarding, long commands, staging, builder config, inspection |
| Tauri | thin adapters, command registration, resource/permission configuration |
| Frontend | chat microphone control, dynamic status, OCR modes, new code panel/sidebar |
| Tests | protocol, dispatcher, runtime neutrality, resource staging, fixtures, diagnostics |
| Documentation | README, doctor, TEST_PLAN, privacy, model licenses, provenance |

## 15. Test and verification matrix

Every phase must run the existing suites first and preserve the baseline.

| Layer | Automated verification |
| :--- | :--- |
| Pure module | normalization, parsers, VAD state machine, geometry, repetition, UID/fingerprint rules |
| Core service | availability, lazy load, cancellation, progress ordering, operation transitions |
| Protocol | every advertised method dispatched; every event allowlisted/correlated; binary WAV preserved |
| Renderer | runtime neutrality, mic state reducer, transcript composer behavior, artifact pagination |
| Persistence | staging/commit failure, compensation, re-analysis, shared provenance deletion |
| Live diagnostics | fixture STT, Supertonic WAV, generated PDF OCR, fixture repository analysis |
| Packaging | required resources/licenses present; forbidden Python/helpers absent; execution permissions valid |
| Manual | microphone/device changes, Turkish audio quality, avatar behavior, OCR visual review, graph navigation |

CI should separate fast deterministic tests from opt-in live model tests. Live
tests must state required model, expected disk/RAM/VRAM, and cleanup behavior.

## 16. Performance and reliability budgets

Track cold and warm measurements separately.

- STT: RTF below 1 on minimum CPU; final event within one second of endpoint;
  bounded audio queue with explicit overflow telemetry.
- TTS: RTF below 1 on minimum CPU; no cancelled generation reaches playback;
  model loaded once and released on backend shutdown.
- OCR: bounded page concurrency; no more than configured Ollama slots; resume
  from page artifacts; memory does not scale with all rendered pages at once in
  the final implementation.
- Code analysis: bounded file/byte/entity/request counts; deterministic mode
  completes without network; semantic failure never discards deterministic
  facts.
- Protocol: remain below 64 MB per frame by using artifact IDs/bounded reads.
- Startup: STT/TTS models load lazily; absent optional models do not delay or
  fail Ayla startup.

## 17. Rollout and feature flags

Use independent flags so one module cannot block the release of another:

```text
speech.stt.enabled
speech.stt.repair_mode
speech.tts.engine
ocr.engine = legacy | glm
ocr.assembly_mode = off | structured | structured_kb
code_analysis.enabled
graph.schema_v2.read / graph.schema_v2.write
```

Recommended rollout:

1. internal diagnostics;
2. opt-in UI with telemetry limited to local logs and no source/transcript text;
3. Supertonic and GLM local extraction defaults after quality gates;
4. STT default-visible but user-started;
5. graph v2 dual-read/controlled write;
6. code analyzer opt-in;
7. remove legacy engines/adapters only after one successful release cycle.

## 18. Main risks and mitigations

| Risk | Impact | Mitigation |
| :--- | :--- | :--- |
| CUDA setting with CPU sherpa build | STT fails at runtime | CPU-only MVP; capability detection; separate CUDA package/profile |
| Microphone packaging/runtime variance | STT unavailable on some systems | packaged cpal spike, explicit device errors, fixture transcription independent of mic |
| Audio feedback from TTS | false commands/transcripts | half-duplex default, stop/suppress policy, headset mode later |
| Cloud transcript or source leakage | privacy breach | local/off default, explicit consent, visible mode, tests proving no call |
| Supertonic license ambiguity | redistribution violation | authoritative license review before bundling; notices and acceptance record |
| OCR toolchain/Poppler drift | build/package regression | edition/MSRV normalization; PDFium geometry/rendering |
| OCR/LLM concurrency exhausts GPU | chat instability/OOM | inference coordinator, bounded modes, hardware-safe contexts |
| Detached native/blocking work ignores cancel | CPU/GPU and temp leak | service-owned tokens, native callback cancellation, join all tasks |
| 64 MB IPC frame exceeded | failed large document/repository | durable artifacts and bounded reads |
| Name-keyed graph collisions | corrupted code graph | graph v2 UID migration before structured code import |
| Re-analysis duplicates knowledge | noisy graph/retrieval | stable content/revision fingerprints and run-aware upserts |
| Remote Git uses credentials or hangs | security/UX problem | disabled by default, HTTPS allowlist, no prompt, timeout, size limits |
| Generic re-extraction loses analyzer facts | quality/provenance loss | direct structured import; Markdown remains preview/retrieval artifact |
| Large models inflate installer | poor distribution | optional verified downloads or explicit offline package flavor |

## 19. Definition of done

The integration is complete only when:

- Ayla runs without any of the four sibling repositories;
- Electron packages exactly one Rust executable;
- no Python runtime or Poppler executable is required;
- every optional model has verified files, license metadata, status, and a
  repair/reinstall path;
- STT, TTS, OCR, and code analysis expose cancellable, tested core services;
- the renderer remains runtime neutral and sandboxed;
- local/cloud boundaries are explicit and local is the safe default;
- preview data is not ingested before approval;
- code analyzer nodes/relations retain stable UIDs, properties, provenance,
  confidence, and source locations;
- re-analysis and deletion do not create duplicates or destroy shared facts;
- package diagnostics pass on a clean install;
- existing chat, memory, search, OCR, graph, TTS, Electron, and Tauri behavior
  remains covered by regression tests;
- documentation describes setup, disk/RAM expectations, permissions, privacy,
  failure recovery, and rollback.

## 20. First implementation slice

The safest first coding slice is Phase 0 plus the TTS portion of Phase 1/2:

1. add provenance/license records and decide Supertonic distribution;
2. introduce runtime capabilities and focused speech configuration;
3. create `ayla-tts` with Supertonic config and NFKD normalization;
4. keep the existing `generate_tts` binary contract;
5. add generation cancellation and renderer epochs;
6. retain Piper behind a rollback setting;
7. add resource staging, package inspection, and Turkish fixtures.

That slice exercises the new crate, resource, capability, cancellation,
Electron, Tauri, and test patterns without first taking on microphone capture,
OCR persistence, or graph migration. Those patterns can then be reused by the
larger phases.
