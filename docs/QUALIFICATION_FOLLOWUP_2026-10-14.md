# Qualification follow-up — 2026-10-14

This follow-up applies to the working tree after candidate `8fa9696`. The
AppImage and Arch files under `release/github` still identify that original
candidate. They are not evidence for these changes. The refreshed unsigned portable candidate is staged separately under
`release/followup-2026-10-14`: AppImage, Linux payload, source installer, native
manifest, source manifest, and checksums. All five checksum entries verified;
all 304 build-input hashes match the current implementation. Both portable
tests and the actual extract-and-run AppImage smoke passed with sandboxing.
No release was signed, tagged, pushed, or published.

## Implementation

- Backend API operations now live in `api/{chat,cleanup,code_analysis,documents,
  graph,history,ocr}.rs`, with request/response contracts in `api/types.rs`.
  `api.rs` went from 3,021 to 843 lines; public command names and wire formats
  remain the same. The knowledge writer mutex still belongs to the parent API
  module, so moving operations does not create separate writers.
- Graph-v2 database transactions, paging, and backup retention moved to
  `memory/graph_v2/store.rs`. Validation, conversion, and snapshot logic stay in
  `graph_v2.rs`. The database writer mutex remains shared.
- KB document actions and bounded node/edge tables moved to `ui/kb/`. The
  remaining canvas/controller code is still large and merits a separate,
  behavior-tested extraction; this pass does not claim that refactoring is done.
- The Compose policy keeps explicit startup after mounting the project disk and
  `create_host_path: false`. README now records startup, stop, and remount behavior.
  The user's storage and existing Compose changes were preserved.
- npm's development audit fell from 14 advisories to zero by selecting
  `gh-pages` 6.1.1 for Live2D's unused publication helper and `global-agent`
  4.1.3. Electron and electron-builder versions stay the same. Both the proxy
  library and electron-builder's downloader proxy initialization were exercised.
  Installed dependency notices are regenerated when staging the package.
- Research diagnostics preserve configured cloud consent. Their previous
  unconditional consent override was removed. This follow-up uses local
  `gemma4:e4b` research with cloud consent disabled.
- A read-only migration review reports counts, pending operations, retained
  snapshots, and embedding metadata without retrieving private content or
  modifying storage.
- GLM token-loop failures now use the documented text-recognition prompt within
  the existing retry limit, with recovery-only repetition settings. Recovery emits a review warning; other models and
  unrelated server errors retain their existing prompts. See the
  [upstream task prompts](https://github.com/zai-org/GLM-OCR/blob/main/glmocr/config.yaml).
  The OCR cache identity is bumped so old warning policies are not reused.
- OCR now consumes bounded streaming responses, stops detected output loops,
  and preserves usable partial text with an incomplete-generation warning.
  Incomplete pages are excluded from the completed-page resume cache.
- OCR now warns about repeated short lines without deleting or rewriting them;
  a regression test confirms text preservation and no automatic retry. The
  public corpus also reports missing expected Turkish words and repeated lines.
- Desktop diagnostics use disposable storage and public fixtures. A regression
  test exercises the actual Pixi ticker at a deterministic 180 Hz clock.

## Current verification

Live prerequisites: isolated SurrealDB and Qdrant from `compose.test.yml`, local
Ollama, and existing loopback SearXNG/Lightpanda. The integration scripts use
fresh database names, vector prefixes, and temporary artifact roots. The user's
knowledge database is not an integration target.

| Check | Result |
|---|---|
| Frontend test/build | 82 tests passed across 23 files; production Vite build passed |
| Rust workspace | 147 passed; three service-dependent tests ignored |
| npm audit | Zero advisories across production and development dependencies |
| Isolated startup | Storage ready, empty namespace, clean shutdown |
| Documents | Approval/restart/retrieval, durable deletion, failed-approval recovery, embedding identity guard, rollback exclusion passed |
| Coordinated backup | Four snapshot files verified; fresh restore/retrieval passed; original test store preserved |
| Code analysis | Approval/retrieval, revisions, rollback, vectors/artifacts deletion and clean shutdown passed |
| GLM OCR | Generated PDF, 42.75 MPa, page/figure evidence, approval/restart/retrieval/removal passed; no temporary outputs remained |
| Features | TTS WAV, chat streaming/cancellation, research cancellation, ingestion/removal and clean shutdown passed with local research |
| Local research | Knowledge complete (12 sources); papers partial (66 sources/six PDFs); history and URL/hash provenance survived restart |
| Source installer | Actual build/install/status/uninstall passed in a disposable path containing spaces; configuration preserved |
| Backend endurance | Passed 30 minutes: 160 cycles, 27 speech runs; descriptors stayed at 18; no pending requests; clean shutdown |
| Public OCR corpus | All three critical numeric-token checks passed with streaming; Turkish word errors and incomplete-output warnings remain human review items |
| Unpacked Electron package | Build and resource/model/runtime-import inspection passed; portable build also exercises installed renderer and actual AppImage runtime |

The initial feature check rejected cloud research because consent was disabled.
The rerun explicitly selected local research and passed. A consent rejection is
not bypassed by a diagnostic.

## Desktop performance

Reference: AMD Ryzen 5 7600, current Linux Wayland session, Electron 41.10.7,
actual 3440×1440 viewport, Auto quality at 0.75 renderer resolution, Mk6 avatar
loaded, 1,000 public fixture nodes and 2,000 edges. Concurrent model/build work
was present; these short measurements are not a mixed-workload desktop soak.

Pixi 6 floors elapsed time to integer milliseconds before comparing it to a
fractional FPS period. A 60 FPS setting therefore waits for at least 17 ms;
on a 180 Hz clock this produces avoidable long intervals. Ayla now selects an
integer limiter period (16 ms for the nominal 60 FPS target, maximum 62.5 FPS).
Resize uses a 33 ms period for its nominal 30 FPS target. Playback timing remains
based on elapsed time. Other refresh rates need their own acceptance recordings.

| Scenario | Baseline FPS | Baseline p95 | Candidate FPS | Candidate p95 |
|---|---:|---:|---:|---:|
| Idle avatar | 58.37 | 22.0 ms | 60.38 | 17.7 ms |
| Synthetic text updates | 58.48 | 20.9 ms | 60.77 | 17.5 ms |
| Classic knowledge graph | 58.55 | 21.5 ms | 60.57 | 17.6 ms |
| Relationship Gravity | 57.79 | 22.1 ms | 60.60 | 17.4 ms |

These are actual renderer tick measurements over ten seconds per scenario.
Browser requestAnimationFrame scheduling is recorded separately in the raw
reports; its approximately 180 Hz rate is not avatar renderer FPS. Neither
measurement proves compositor presentation timing. Synthetic text updates
exercise DOM pressure rather than real model streaming. Relationship Gravity
finished successfully through the worker in both runs.

Raw evidence: [baseline](evidence/desktop-performance-baseline-2026-10-14.json)
and [candidate](evidence/desktop-performance-candidate-2026-10-14.json).
A separate XWayland run through Electron’s X11 backend measured approximately
60 FPS and p95 renderer intervals of 17.3–17.4 ms at the same viewport.
[Raw XWayland evidence](evidence/desktop-performance-xwayland-2026-10-14.json)
records the requested backend. This is not an independent native X11 session.

The build remains approximately 555 KB for the entry JavaScript and 487 KB for
Markdown before gzip. This pass fixes measured frame pacing; it does not claim a
bundle-size reduction or hide the build warning.

The 30-minute backend run completed 160 cycles and 27 speech runs. Post-warmup
RSS was 242,844 KiB; final RSS was 166,232 KiB; peak was 303,968 KiB.
Descriptors stayed at 18. Host paging and concurrent workloads affect resident
figures; this is backend endurance evidence, not a desktop or microphone soak.

## Public OCR review corpus

The generated bilingual-column, table/equation/figure, and rotated-raster PDFs
all preserved their required numeric/unit tokens after streaming recognition.
The bilingual and raster cases still missed the exact Turkish word `Şişe` and
reported incomplete generation. Repeated headings remain review signals.
These are unresolved language/reading-order quality issues, not successful human
acceptance. Earlier full-response requests hit Ollama's token-loop abort; the
streaming pipeline now preserves useful partial text and leaves incomplete
pages out of the completed-page cache.

[Corpus report](evidence/ocr-corpus/review-results.json),
[reference manifest](evidence/ocr-corpus/manifest.json), and the recognized
Markdown alongside them contain only generated public content. Recreate the
PDFs with `npm run ocr:fixtures`; they remain local under `release/ocr-corpus`.
[Structured qualification results](evidence/qualification-2026-10-14.json)
record the other integration outcomes and the user-deferred microphone gate.

## Existing-data observation

Read-only review on October 14 found 915 graph nodes, 1,944 edges, six documents,
no failed/interrupted/deleting operations, no detected unscoped legacy
facts/episodes, and seven retained graph snapshots. Embedding identity is
`qwen3-embedding:0.6b`, 1,024 dimensions, with a recorded digest. This is a live
aggregate observation, not a backup or a verified migration.

Unique generic filenames and full snapshot graph writes remain current
constraints. Path identity migration needs collision handling across receipts,
text storage, vectors, citations, graph provenance, and rollback. Incremental
writes need transaction/recovery guarantees and deletion/rollback parity. Neither
should be replaced by an unqualified schema change on the user's database.

## Remaining acceptance

- Human review of OCR reading order, tables/equations, units, and real page
  evidence across Turkish/English scans, rotations, and malformed input.
- Human research claim/citation review, contradictory sources, and partial-result
  interpretation. Automated URL/hash checks do not establish factual quality.
- Real microphone/device switching, permission denial, background noise, and
  Turkish/English transcription and speech intelligibility. The user explicitly
  deferred microphone acceptance during this follow-up.
- Additional GPU/driver and native X11 acceptance; real streaming plus concurrent graph,
  OCR/research, and a mixed desktop soak. Wayland color-management warnings were
  emitted by Chromium on this host and need visual acceptance.
- A coordinated private backup and fresh-target restore of user-owned data before
  any actual schema migration; retained snapshot policy review.
- Refactoring the remaining KB canvas controller and separately designed source
  identity/incremental graph migrations.
- Final native Arch rebuild, source/license review, reviewed immutable commit,
  signing/tag/checksums, publication, and downloaded-artifact acceptance.

Only the isolated `ayla-release-tests` containers were removed after verification.
The user’s production containers and knowledge storage were preserved.
