# Ayla first production release — improvement and delivery plan

Date: 2026-10-13

Status: Linux candidate implementation complete; final qualification evidence and publication gates are recorded separately

Target: Linux x86_64, Electron + Rust only

Delivery: AppImage and a universal Linux installer; native Arch and AUR packages also remain in scope. No Debian package.

## Implementation status and qualification boundaries

The initial audit below is historical evidence. Current outcomes and exact
candidate results are recorded in [release evidence](RELEASE_EVIDENCE.md).

| Area | Implemented outcome | Qualification remaining |
|---|---|---|
| Runtime/build | Electron-only host; MessagePack packaged; pinned native bootstrap; qualified Debian 12 and Arch builds | Maintainer review, tag and signature |
| Knowledge | Authoritative Graph-v2 storage; native code/OCR retrieval; durable approval/deletion receipts; shared evidence; rollback exclusion | Large existing-data migration review |
| Backup | Quiesced coordinated export, checksums, fresh restore, vector/text retrieval verification | User-owned snapshot retention policy |
| OCR | GLM/PDFium preserved; coherent approval; model-digest cache; rendering/cache bounds; cancellation cleanup | Human-reviewed multilingual/scanned/table/equation corpus |
| Research | Independent model/consent; guarded public networking; streamed bounded PDFs; durable stage and partial reports | Human claim/citation accuracy review |
| Lifecycle | Avatar-free panels, sandbox, writer lock, service recovery, bounded logs/support export, bounded document cache | Real devices/compositors and desktop performance |
| Distribution | AppImage, versioned source installer, source/bin Arch recipes and metadata; CI for portable and Arch builds | Published tag/URLs, signatures, AUR submission and downloaded-artifact acceptance |

First-candidate constraints are deliberate and documented: Linux x86_64/glibc
2.36 baseline; unique generic document filenames; snapshot graph writes; public
HTTP fallback for JavaScript-only pages; no licensed avatar/Cubism payload and
no unreviewed Piper model. Rich job/snapshot browsing and incremental global
path identities remain follow-up work. Historical unscoped facts/episodes are
not automatically purged because their source cannot be established safely.

A passing automated candidate is not a claim that human evidence-quality,
real microphone, GPU/frame-time, Wayland, or maintainer redistribution/source
checks were performed. Those remain explicit publication gates. Publication,
signing, and AUR upload are outside the user's requested local implementation.

## 1. Objective and order

Deliver a first production package that installs without the repository, starts reliably, preserves user data, and provides verified chat, memory, OCR, research, code analysis, and speech workflows.

Improve and verify the application first. Complete Tauri removal before the release candidate. Build installers and publishable packages as the final implementation phases. Package structure and resource correctness must be considered earlier because they affect application architecture, but distributable creation is deliberately last.

The first supported architecture is x86_64. “Universal Linux installer” means one distro-independent installation workflow for a documented Linux compatibility range; it does not promise support for every architecture, libc, distribution, or desktop. Establish that compatibility range through testing before release.

### Already implemented upgrades to preserve

| Area | Current implementation | Release work needed |
|---|---|---|
| OCR extraction | In-process `ayla-ocr`, PDFium rendering and text geometry, `glm-ocr:latest` default | Real quality corpus, bounded resource use, packaged native-library verification |
| OCR reliability | Page workers clamped to 1–8, resume cache, transport/quality retries, repetition cleanup, deterministic page ordering | Failure recovery, cancellation at every stage, cache invalidation and cleanup checks |
| OCR outputs | Markdown, layout JSON, page-cited KB document, structured graph artifact | One coherent approval lifecycle and retrieval integration |
| OCR semantic assembly | Separate assembly model, loopback endpoint requirement, cloud consent, deterministic fallback | Verify local/cloud modes, evidence grounding, explicit partial-success UI |
| Research | In-process `ayla-research`, SearXNG, PDF download, Lightpanda, analysis/ranking/synthesis | Source accuracy, bounded downloads, durable job state, partial-failure handling |
| Model concurrency | Research uses one concurrent analysis for local models and two for cloud-tagged models | Test hardware pressure and coordinate concurrent chat/OCR/research |
| Speech | Supertonic 3 INT8 Turkish/English TTS; Omnilingual 300M INT8 + Silero VAD STT | Packaged model loading, microphone permissions, cancellation, real audio validation |
| Code intelligence | Deterministic Tree-sitter analysis, optional semantic enrichment, run history, Graph-v2 approval and dedicated vectors | Make approved knowledge available to chat and ensure complete deletion |

The audited local configuration selects `nemotron-3-super:cloud` for chat and research, the same model for code semantic analysis, `glm-ocr:latest` for OCR, and Nemotron cloud for semantic OCR assembly. Code/OCR cloud consent is enabled locally. These are this checkout's configuration, not defaults that should silently become every user's settings.

At the initial audit, research inherited `AYLA_LLM_MODEL`. It now has independent `AYLA_RESEARCH_MODEL` selection and an explicit `AYLA_RESEARCH_ALLOW_CLOUD` gate. OCR assembly already has an independent setting. The configured local context ceiling is 256,000, while automatic context buckets top out at 131,072; effective context selection must be documented and tested rather than assumed from the configured number.

### Initial audit evidence (before implementation)

- Clean working tree before this planning document.
- Frontend test runner: 18 passing test files; production Vite build succeeds.
- Rust workspace: 129 passing tests, 3 service-dependent tests ignored.
- Existing Linux resource inspection passes, including speech-model hashes.
- Offline sidecar diagnostic fails immediately: 49 expected commands versus 50 advertised commands.
- Existing `app.asar` imports `@msgpack/msgpack` but contains no MessagePack package. The dependency is currently under `devDependencies`, and the file rules exclude `node_modules`. This is a concrete packaged-runtime blocker that file/resource inspection misses.
- Ollama was reachable; configured SurrealDB, Qdrant, SearXNG, and Lightpanda ports were closed. End-to-end persistent workflows remain unverified.
- No tracked CI workflow was found. Existing local binaries and release files are not evidence of a reproducible clean build.

## 2. Release blockers and priorities

P0 means the first release cannot ship with the issue unresolved. P1 means required within the release scope, but work follows the foundational corrections.

| ID | Priority | Problem | Required outcome |
|---|---|---|---|
| R01 | P0 | Packaged Electron runtime dependency omitted | Installed main process resolves every runtime import without checkout dependencies |
| R02 | P0 | Legacy graph and Graph-v2 have different producers/consumers | All supported knowledge flows use one authoritative graph and source identity |
| R03 | P0 | Code-analysis vectors and native Graph-v2 knowledge are absent from chat retrieval | Approved code/OCR knowledge is retrievable with citations |
| R04 | P0 | Document deletion does not persist text removal and can ignore vector failures | Durable, retryable deletion with no silent leftovers |
| R05 | P0 | Metadata, vectors, graph, and approval state commit separately | Recoverable ingestion/deletion state machine and idempotent operations |
| R06 | P0 | Clean checkout requires ignored Sherpa-ONNX source | Pinned and documented dependency acquisition; verified clean build |
| R07 | P0 | Tauri packages, runtime branches, resources, tests, and diagnostics remain | Complete Electron-only application and dependency trees |
| R08 | P0 | Avatar/core loading can abort feature initialization | Application works without licensed avatar assets or network access |
| R09 | P0 | Live and installed workflows are not covered by passing unit tests | Automated integration evidence and clean-machine acceptance matrix |
| R10 | P0 | OCR provenance document records an unresolved upstream license | Establish provenance and redistribution rights; replace unlicensed copied implementation if needed |
| R11 | P1 | Degraded persistence is fixed at startup | Recover after services become available without losing pending durable work |
| R12 | P1 | Research failures and partial outputs can appear complete | Clear outcome states, source failure reasons, recoverable results |
| R13 | P1 | Documentation and diagnostics refer to old names and helpers | One current setup, support, and release procedure |

## 3. Phase 0 — establish a reliable baseline

1. Record the release commit, toolchain versions, Node/Electron versions, service images, model identifiers, model digests where available, and current feature contracts.
2. Inventory commands/events from the shared Rust catalog. Replace the stale diagnostic count with catalog/capability checks; a command-count-only test must not substitute for route coverage.
3. Run service-dependent diagnostics against dedicated test namespaces, databases, vector collections, and temporary artifact roots. Existing diagnostics that mutate default storage must be isolated before reuse.
4. Build a representative fixture set: Turkish/English chat, PDFs, scanned documents, research pages, code projects, speech recordings, and shared-provenance documents. Record deterministic expectations and model-dependent review criteria separately.
5. Create a release evidence directory or CI artifact convention containing command outputs, versions, test reports, installed-package logs, and unresolved defects. Exclude secrets and user content.
6. Label historical plans as historical. Make this document the first-release execution plan; retain the Graph-v2 cutover plan as detailed design input.

Acceptance: the diagnostic command catalog matches the backend; baseline checks are reproducible; live prerequisites are explicit; test mutations cannot affect the user's actual knowledge base.

## 4. Phase 1 — knowledge consistency, retrieval, and data safety

### 1A. One graph and stable source identities

- Move graph-neutral entity/relation types out of legacy `graph.rs`.
- Record stable IDs for document receipts, research sessions, OCR operations, and code runs. The first candidate retains unique filenames as the generic workspace compatibility identity and rejects colliding paths; a full path-based workspace identity migration remains follow-up work.
- Adapt every producer to a provenance-preserving Graph-v2 import: generic ingestion, Search, Chat-to-KB, automatic conversation memory, tools, OCR, and Code Analyzer.
- Move retrieval neighbors, consolidation, graph refinement/cleanup, deletion, pagination, and UI reads to Graph-v2.
- Track incremental graph writes as follow-up performance work. This candidate uses serialized validated snapshots with retention limited to 25 backups; benchmark large graphs before declaring their supported scale.
- Make Graph-v2 read errors visible. Remove silent fallback to the legacy graph after cutover.
- Migrate existing legacy sources once, with a schema version and a resumable migration record. Preserve native Graph-v2 UIDs and shared evidence. Remove startup mirror behavior after verified migration.
- Remove legacy graph storage/API/UI paths once migration tests and restored-data checks pass. Do not require users to erase their databases to upgrade.

### 1B. Make all approved knowledge retrievable

- Extend hybrid retrieval to Graph-v2 and the code-analysis vector collection alongside documents, facts, and episodes.
- Normalize result metadata: source ID, run ID, citation, chunk type, confidence, and approval status.
- Deduplicate and rank results across stores; apply a bounded context budget. Preserve original evidence separately from model-generated concepts.
- Exclude previews, unapproved runs, failed commits, and deletion-pending sources.
- Validate embedding dimensions at collection initialization. Record model identity/digest with indexed data and provide explicit reindexing when the embedding model changes.

### 1C. Coordinated commits and deletion

- Define a durable job/source lifecycle such as `preview → approving → committed`, plus `failed`, `deleting`, and `deleted` states.
- Use SurrealDB transactions within that store; use a durable operation journal and compensating/retry steps across SurrealDB, Qdrant, and filesystem artifacts. Do not claim cross-service atomicity.
- Consolidate file approval and graph ingestion behind one backend operation with an idempotency key. The UI receives a final commit receipt only after all required stores agree.
- Replace whole-table text-store rewrites with per-source updates/deletes. Persist deletion even when the last cached document is removed.
- Treat vector deletion failure as incomplete deletion, with retry/reconciliation. Hide deletion-pending sources from retrieval immediately.
- Delete provenance from Graph-v2, text records, vector collections, owned artifacts, and related approval state consistently. Preserve shared knowledge and original user files.
- Define retention separately for deleted content, content-bearing backups, and minimal audit receipts; make retention visible. A deletion receipt must not imply backup content was erased when it was retained.
- Refresh KB, Search, OCR, Code, and Chat from a common commit/delete event. Reset stale Search “In KB” flags when its source is removed.
- Add single-instance application behavior and a backend/data-writer lock keyed to the persistent store/workspace. Prevent concurrent migration/restore/write owners.

### 1D. Minimum reliable backup and migration protection

- Provide coordinated export/restore for graph, document records/text, source/run metadata, facts/episodes, vectors or verified reindexable inputs, and owned artifacts.
- Quiesce writers while capturing/restoring state. Include a checksummed manifest and schema versions; validate before modifying live storage.
- Back up before migration; restore into isolated storage first and verify counts, provenance, citations, and retrieval.
- Keep graph-only rollback clearly labeled and scoped until it is made consistent with the other stores.
- A rich snapshot browser with pinning and comparison is deferred; safe migration backup and tested restoration are release requirements.

Acceptance: approve a document/code/OCR run, restart, retrieve it in chat, delete it, restart again, and confirm it is absent from every active store. Shared evidence survives deletion of one supporting source. Inject failures between commit stages and verify reconciliation. Restore a backup and verify retrieval, not just graph counts.

## 5. Phase 2 — validate the upgraded OCR system

1. Preserve GLM/PDFium and the existing semantic pipeline. Remove obsolete DeepSeek/helper instructions from active documentation.
2. Record the effective extraction model, assembly model, consent, context, page range, processing options, and fallback outcome for each run. Cache validity must include all output-affecting configuration, including model identity changes.
3. Expose deterministic versus semantic processing and local versus cloud assembly clearly. Never silently inherit chat cloud consent for document/code processing.
4. Integrate OCR approval with Phase 1's coordinated commit. The user reviews one evidence package and approves the document/graph/vector contribution together; low-level artifact export remains separate.
5. Validate born-digital, scanned, rotated, two-column, mixed-language, table-heavy, equation-heavy, and low-quality PDFs. Include encrypted, malformed, empty, oversized, and very long documents.
6. Score factual evidence preservation, page citation accuracy, column reading order, table structure, number/unit retention, and hallucinated content. A non-empty string is insufficient success criteria. Freeze reviewed tolerances with the corpus before final release testing.
7. Test resume after cancellation/crash, partial-page failures, transport retries, model unavailability, disk exhaustion, and cloud denial. Preserve deterministic evidence if optional semantic assembly fails; display the failure explicitly.
8. Bound PDF dimensions/render buffers, page concurrency, cache size, artifact size, and retained memory. Establish conservative CPU/GPU defaults; coordinate with chat and research to avoid competing model loads.
9. Make cancellation prompt at render, OCR, assembly, and persistence stages. Mark incomplete runs and remove abandoned temporary files; retain only intentional resume cache.
10. Verify PDFium and speech/native libraries outside the checkout, with no developer library-path dependence.

Acceptance: reviewed corpus results meet the frozen tolerances; all citations refer to real pages; cancelled/failed runs cannot become approved silently; retries/resume do not duplicate content; both deterministic/local and explicitly consented cloud modes are verified.

## 6. Phase 3 — improve search and research reliability

1. Preserve in-process SearXNG/Lightpanda discovery, PDF extraction, model analysis, ranking, and synthesis.
2. Add a dedicated research model setting with an explicit backwards-compatible fallback to the chat model. Record the actual model per session and allow local/cloud selection independently where needed.
3. Show the cloud data scope before enabling cloud research: query, selected source text, and generated evidence. Existing local consent choices must be preserved during upgrade, not reset or silently expanded.
4. Persist job stages and per-source outcomes: queued, fetching, extracted, analyzed, synthesized, partial, failed, cancelled, and completed. Retain useful results when synthesis fails; do not mark failed synthesis as a fully successful report.
5. Validate citation grounding, URLs, DOI/title metadata, source identity, duplicate PDFs, contradictory findings, and inaccessible sources. Mark unsupported claims and distinguish quoted evidence from model inference.
6. Use a shared public-URL policy for knowledge fetching, PDF downloads, and rendered-page navigation, including redirects and resolved destinations. The knowledge-search path already has restrictions; audit and unify the paper paths rather than assuming parity.
7. Stream downloads with enforced byte limits. Current PDF downloading buffers the entire response before checking its actual length; a missing/false Content-Length must not permit unbounded memory use. Write to temporary files and atomically finalize validated PDFs.
8. Bound discovery, downloads, rendering, text retention, analysis concurrency, and total job storage. Handle timeouts, empty results, malformed JSON, invalid PDFs, paywalls, missing Lightpanda, cloud authentication/rate limits, and offline Ollama.
9. Make cancellation and renderer ownership intentional: explicitly define which jobs continue in the background and which stop on user cancellation, application shutdown, or sidecar crash. Reconcile abandoned jobs after restart.
10. Feed research approval/deletion through the shared source lifecycle. Keep reports/downloads available independently of KB approval, and remove only Ayla-owned files when deletion requests include them.
11. Add controlled HTTP/CDP/model fixtures for repeatable regression tests plus a small live smoke suite. Freeze a reviewed set of general-knowledge and scientific topics for final evidence-quality checks.

Acceptance: both search modes finish with accurate outcome states, source-linked reports, persistent history, and retrievable approved knowledge. A failed source or failed synthesis is visible. Retry/cancellation do not duplicate history or KB sources.

## 7. Phase 4 — runtime, onboarding, security, and performance

### Startup and service recovery

- Initialize feature panels independently of Cubism/avatar loading. Missing avatar assets or failed core loading must leave chat, status, OCR, search, and settings usable.
- Ship a lawful local Cubism runtime if redistribution is permitted; otherwise make avatar activation an explicit optional installation with a working avatar-free mode. Do not fetch executable UI code silently at every startup.
- Detect service and model capabilities individually. Distinguish backend connected, storage ready, model available, cloud authorization, and search service ready.
- Replace permanently offline startup state with explicit recovery/reinitialization. Preserve unfinished jobs and block persistence-dependent operations until safe to resume.
- Add installed settings for endpoints, credentials, models, consent, resource directories, and processing limits. Packaged builds must not depend on a repository `.env`.
- Validate effective context against model capabilities; surface the real applied limit. Validate embedding compatibility and schedule explicit reindexing on change.
- Treat services as explicit prerequisites. Provide a versioned local-services Compose bundle and setup diagnostics; do not require Docker access for features that do not use it or start root/system services implicitly.

### Process and data lifecycle

- Verify sidecar handshake, restart limits, owner cancellation, binary backpressure, overload handling, and shutdown under active workloads.
- Keep native speech/inference and blocking parsing off async/runtime and UI threads where needed; use bounded workers.
- Persist user data under stable user directories shared by AppImage, installer, and Arch installs. Confirm application identity and directory paths before release.
- Use rotating logs and a redacted support bundle. Include versions, model identities, job/error IDs, and capability status; exclude credentials, raw private documents, and microphone recordings by default.
- Keep existing Electron isolation/sandboxing and Markdown sanitization. Review IPC authorization, external-link schemes, artifact path containment, resource symlinks, and the broad localhost Ollama proxy surface. Restrict proxy routes/methods or move operations to validated backend APIs.
- Add meaningful XSS/IPC/path/network regression tests based on actual attack surfaces, alongside oversized/malformed protocol tests.

### UI and performance

- Register only needed Highlight.js languages or isolate loading to reduce the 1.28 MB Markdown chunk; investigate the 693 KB entry chunk. Preserve mathematical and language rendering correctness.
- Keep panel loading lazy, graph layout in the worker, render budgets, visibility throttling, and cancellation responsiveness.
- Measure real idle, streaming, graph, and OCR/search workloads on declared reference hardware. Set numerical CPU/RAM/frame-time tolerances from those measurements.
- Retain the planned 3440×1440 target: approximately 60 FPS idle, p95 frame time at most 20 ms on the declared reference machine, with no sustained collapse during streaming.
- Perform a 30-minute mixed-workload soak and repeated open/close/restart cycles. Require bounded listeners/processes/queues/memory after warmup and no orphan sidecar.

Acceptance: cold start, absent avatar, absent services, later service recovery, model failures, denied microphone, and active-work shutdown produce understandable states. No required feature initialization depends on avatar success. Work remains responsive and data is durable.

## 8. Phase 5 — clean builds and release verification foundation

1. Pin a verified Rust toolchain/MSRV and Node version; test the claimed minimum or raise it. Commit dependency locks and use `npm ci` and locked Cargo builds in CI.
2. Replace the ignored Sherpa source assumption with pinned crates, a tracked vendored dependency, or a verified bootstrap source archive. Pin native downloads with digests. Capture PDFium/ONNX/speech runtime linkage and required libraries.
3. Fix MessagePack packaging: make it a production dependency and include its required files, or bundle Electron main modules into a self-contained output. Add archive import-resolution validation and a real installed launch check.
4. Build in an isolated environment with no `node_modules`, `target`, local release directory, ignored avatar, developer `.env`, or preexisting speech source. Repeated builds should use the same locked inputs; byte-identical output is a separate goal, not an assumed property.
5. Add CI jobs for frontend checks, Rust tests, service/mock integration, protocol diagnostics, archive/resource checks, and a graphical packaged smoke test where supported.
6. Inventory app/dependency/model licenses and notices. Resolve the OCR provenance issue, add the application license file, and verify Cubism/avatar and every included speech asset, including Piper, before distribution.
7. Pin service image versions/digests, including Lightpanda's current `latest` reference, and document tested upgrade/restore procedures.
8. Define version/schema/protocol compatibility and minimum supported Linux environment. Build AppImage native components on a suitable compatibility baseline, not only the newest developer Arch system; confirm by clean-machine launch tests.
9. Establish the candidate evidence format now; freeze the candidate only after the final Tauri removal in Phase 6 and a repeat of clean-build verification. Record all known limitations and any excluded optional features explicitly.

### Mandatory workflow matrix

| Workflow | Required evidence |
|---|---|
| Chat | Streaming, cancellation, history restart, model switch, cloud/local errors |
| Knowledge | Approve, retrieve with source citation, shared evidence, durable delete, failed-commit recovery |
| OCR | Corpus quality, deterministic/semantic modes, consent, caching/resume, cancellation, artifact/KB persistence |
| Research | Both modes, source/report accuracy, partial failures, downloads, cancellation/restart, approval/deletion |
| Code | Directory/snippet, deterministic/semantic modes, diff, import, chat retrieval, complete run deletion |
| Speech | Turkish/English playback and real dictation, stop, permission denial, missing model, bounded recording |
| Migration/backup | Existing-data migration, interruption recovery, verified restore and retrieval |
| Lifecycle | Startup offline, service recovery, second instance, sidecar crash, shutdown during every long job |
| Installation | Standard user, checkout absent, desktop launch, paths with spaces, update, rollback, uninstall preserving data |

Acceptance: all required automated suites pass; model-dependent workflow results are reviewed; isolated clean builds succeed; evidence identifies the exact candidate inputs. No P0 defect remains.

## 9. Phase 6 — complete Tauri removal (final application cleanup)

This is required, not optional reference-host retention. Git history is sufficient for historical recovery; the shipped and maintained application contains no Tauri host.

Perform this after the application improvement phases and before creating distributables. Repeat the full clean-build and workflow gates after removal; earlier Tauri-era results do not qualify the final candidate.

### Exact removal inventory

- Delete `src-tauri/`, including its Rust package/lockfile, build script, commands, capability configuration, icons, and auxiliary binary.
- Remove `@tauri-apps/api`, `@tauri-apps/cli`, dialog/shell plugins, and the `tauri` npm script. Regenerate the npm lockfile from the final dependency set.
- Remove the `src-tauri` workspace exclusion and obsolete ignore entries/build assumptions.
- Simplify `src/lib/ipc.js` to Electron transport plus an explicitly limited browser-development mode. Remove Tauri globals, imports, Channels, dialogs, event branches, and shell branches.
- Remove Tauri avatar loading in `src/live2d/mk6-profile.js`, associated blob/resource code, and related tests. Preserve Electron's local model endpoint.
- Remove Tauri status labels and host selection from active UI.
- Change `AYLA_DEVELOPMENT_ROOT` to the repository root in Electron main, all diagnostics, and the Rust sidecar default. Update speech resource paths from `../tts`/`../models` to `tts`/`models` together.
- In installed mode, resolve required assets from installed/configured locations only. Missing packaged resources must produce a clear error rather than silently finding a developer checkout.
- Update resource, STT/TTS, sidecar, IPC, identity, and integration tests that currently reference Tauri paths or read deleted files.
- Rewrite `doctor.sh` for Electron + workspace crates. Remove Tauri checks and hardcoded `root:root` database checks; consume configured credentials without printing them.
- Update active README/setup/test/configuration/provenance documentation. Historical plans may mention Tauri only when explicitly marked historical and containing no active setup instructions.
- Check generated frontend output, archive contents, npm dependency tree, Rust dependency tree, scripts, and diagnostics for Tauri remnants. A string scan alone is insufficient; dependency and runtime checks are required.

Acceptance: clean checkout builds/tests without `src-tauri`; all methods in the current shared catalog have sidecar coverage; Electron chat/events/dialogs/notifications/audio/avatar paths work; no Tauri dependency or runtime branch remains in active code or release payloads.

## 10. Final phase A — AppImage and universal Linux installer

Start distributable implementation after the preceding readiness gates. Use one verified staged payload rather than independently maintained application copies.

### AppImage

- Remove `deb` from electron-builder targets and retire Debian-specific delivery instructions. First-release builds target Linux x86_64 only.
- Include Electron runtime/main dependencies, one Rust sidecar, required native libraries, icons/desktop metadata, notices, and verified speech assets.
- Keep Ollama LLM weights and database volumes outside the application image. Provide model setup/capability checks; preserve existing user settings.
- Keep licensed Mk6 assets outside redistribution unless permission is established; avatar-free first launch is mandatory.
- Test normal AppImage launch and an extracted-payload alternative where image mounting is unavailable. Keep Electron sandboxing enabled; do not adopt `--no-sandbox` as the standard launcher workaround.
- Test on clean Arch and at least one non-Arch supported distribution, with Wayland and X11 sessions where available. Record supported libc/runtime/FUSE conditions and GPU limitations.

### Universal Linux installer

- Provide an inspectable `install.sh` using the same release payload/manifest, with version/channel selection, explicit destination, offline/local-payload installation, verification, desktop integration, update, rollback, and uninstall commands.
- Default to user installation under `~/.local/opt/ayla/<version>`, a stable launcher under `~/.local/bin`, and desktop/icons under user directories. Use XDG-aware settings/data/cache paths independent of install version.
- Use versioned directories and an atomic active-version switch. Do not overwrite a running installation in place. Interrupted download/extraction must leave the previous version launchable.
- Verify architecture and compatibility, payload digest, and release authenticity before execution. Publish a signing public key and a documented verification path; a checksum fetched from the same server alone is not authenticity verification.
- Reject unsafe archive entries and symlinks; avoid writable shared executable directories. Do not download or run code as root for the default user install.
- Make optional system installation explicit. No automatic package-manager edits, Docker-group changes, service activation, credential creation, or model downloads without the relevant setup choice.
- Preserve data on uninstall by default; offer separately explicit removal. Detect package-manager-managed installations and direct users to their package manager instead of overwriting them.
- Report PATH/desktop-session limitations clearly. Do not silently modify shell profiles; provide a copyable launcher path when needed.

Acceptance: online and offline installation, desktop launch, reinstall, interrupted update, rollback, paths with spaces, concurrent install prevention, and uninstall succeed without the checkout. User data and settings survive all install-format transitions.

## 11. Final phase B — Arch Linux and AUR packages

These remain part of the requested Linux distribution scope. AUR delivery consists of build recipes and metadata; built pacman packages are separate release artifacts.

- Add versioned `packaging/arch/` recipes and generated `.SRCINFO`; use a distinctive package name after checking availability (`ayla-ai` is a provisional name).
- Provide a source recipe for the tagged release and a `-bin` recipe for the signed/checksummed official Linux binary payload. Both must install the same application/resources and declare conflicts/provides consistently.
- Build native `.pkg.tar.zst` artifacts through the source recipe in a clean Arch chroot. Run package linting, inspect dependencies/files/licenses, and test installation with pacman.
- Follow Arch filesystem conventions: launcher under `/usr/bin`, application payload under a suitable managed directory, desktop/icons/licenses under `/usr/share`. Package files remain read-only; user state remains under user directories.
- For the source recipe, use the supported system Electron major where feasible under Arch's Electron guidance. Verify compatibility before selecting the dependency. Keep AppImage/installer's bundled runtime choice separate from this recipe. The binary recipe must clearly declare its bundled-runtime behavior.
- Acquire pinned source/native/model archives through declared, checksummed package sources. Avoid hidden network downloads in `build()`, unversioned Git checkouts, and accidental dependency on the developer filesystem.
- Preserve separate model license notices. Whether speech assets are embedded or delivered in a shared model subpackage must be consistent across source/binary recipes and verified in clean installs.
- Declare actual runtime/build dependencies from clean-chroot and runtime evidence. Describe optional Ollama/search/storage service requirements without starting services from install hooks.
- Check AUR naming availability and current submission rules before publishing. Generate metadata for every revision; document `pkgver`/`pkgrel`, source URLs, checksums, and release-update maintenance.
- Prepare and test AUR repositories locally. Uploading to AUR and publishing releases are distinct final actions; this planning task does not publish anything.

Acceptance: source and binary recipes produce valid packages, install/update/remove through pacman, start without the repository, preserve data, and pass the same feature matrix as AppImage. `.SRCINFO` matches each recipe. AUR source metadata references immutable released inputs.

Arch references: [Creating packages](https://wiki.archlinux.org/title/Creating_packages), [Building in a clean chroot](https://wiki.archlinux.org/title/DeveloperWiki:Building_in_a_clean_chroot), [Arch User Repository](https://wiki.archlinux.org/title/Arch_User_Repository). Recheck Electron and AUR submission guidance at implementation/publication time.

## 12. Final phase C — release candidate qualification and publication

1. Re-run the installed workflow matrix for AppImage, universal installer, and Arch source/binary installs using fresh user accounts and production-mode resource resolution.
2. Perform upgrade tests from this development schema and from the previous candidate. Verify migration backup, rollback boundaries, and data-format compatibility.
3. Generate release notes, installation/service/model instructions, known limitations, support diagnostics, checksums/signatures, dependency/license manifest, and reproducible build instructions.
4. Verify that no credentials, developer paths, Tauri binaries/dependencies, test data, or licensed avatar files enter release payloads.
5. Assign the final version and publish the approved release artifacts and tested AUR metadata only after all release gates pass.
6. Smoke-test downloaded artifacts and published installer URLs as a normal user; do not rely exclusively on local build outputs.

Release-ready means: no unresolved P0 defects; full Tauri removal; upgraded OCR/research verified; consistent knowledge lifecycle/retrieval; reproducible clean builds; lawful complete payloads; tested installation/update/uninstall; and evidence tied to the published version.

## 13. Execution sequence and scope control

Implement in reviewable slices with the following dependency order:

1. Baseline diagnostics and isolated fixtures.
2. Stable source IDs and coordinated ingestion/deletion.
3. Graph-v2 producers/consumers, unified retrieval, migration/restore.
4. OCR corpus and failure-recovery improvements.
5. Research model configuration, source handling, bounded downloads, durable jobs.
6. Avatar-independent startup, service recovery, settings, speech/lifecycle/performance verification.
7. Clean-build bootstrap, runtime dependency correction, CI, and license resolution.
8. Complete Tauri removal and resource-path conversion as the final application cleanup; repeat readiness checks.
9. AppImage and universal installer.
10. Arch packages and AUR recipes.
11. Installed candidate qualification and publication.

Runtime dependency correction and license provenance investigation should begin early because either can invalidate later release work. Installer/distributable creation remains at the end. Re-run focused checks after each slice, then run the full matrix when a candidate is formed.

Defer Windows/macOS/ARM support, additional provider integrations, new avatar features, automatic self-updating, additional speech engines, and a rich snapshot-management UI. Existing upgraded OCR/search features are preserved; improving their correctness and deployability takes priority over introducing more models.

## 14. Key implementation files

- Backend API/lifecycle: `crates/ayla-core/src/api.rs`, `state.rs`, `config.rs`, `processes.rs`, `artifacts.rs`.
- Knowledge: `crates/ayla-core/src/memory/{mod,graph,graph_v2,pipeline,ingestion,retrieval,document_store,code_analysis_store}.rs`.
- OCR: `crates/ayla-ocr/src/`, `src/ui/ocr-panel.js`.
- Research: `crates/ayla-core/src/research.rs`, `crates/ayla-research/src/`, `src/ui/search-panel.js`.
- Electron/resources: `electron/{main,sidecar-manager,sidecar-protocol,static-server,stage-resources,inspect-package}.mjs`, `crates/ayla-core/src/resources.rs`.
- Frontend transport/avatar: `src/lib/ipc.js`, `src/live2d/mk6-profile.js`, `src/main.js`, `src/ui/status-panel.js`.
- Build/release: `package.json`, `package-lock.json`, `Cargo.toml`, `Cargo.lock`, `electron-builder.yml`, `doctor.sh`; new CI, installer, and Arch packaging files.
- Supporting design: [Graph-v2 cutover plan](GRAPH_V2_CUTOVER_PLAN.md), [module provenance](module-provenance.md), [model licenses](model-licenses.md).
