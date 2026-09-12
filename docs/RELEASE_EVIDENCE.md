# Linux release candidate evidence

Qualification: 2026-10-13 UTC / 2026-10-14 Europe/Istanbul. Candidate version
0.1.0; protocol 1; knowledge schema 2. Base Git commit `82cb395`;
the original is preserved on the local backup branch. The user authorized a
clean release commit without model binaries; its hash is recorded with final
artifact metadata.
The build's `source-manifest.json` fingerprints the actual copied inputs;
it is the precise candidate identity until a reviewed release commit exists.

## Verified outcomes

| Check | Evidence |
|---|---|
| Frontend | 22 test files passed; production Vite build passed |
| Rust | 142 tests passed; three existing service-dependent tests ignored; same suite also passed inside Debian 12 and Arch builds |
| RustSec audit | 11 vulnerabilities fixed by patched PDF/XML/HTTP/TLS dependencies; zero remaining vulnerabilities. Two unmaintained warnings remain (`rustls-pemfile`, `ttf-parser`) |
| Production npm audit | Zero advisories in installed production dependencies |
| Development npm audit | 14 advisories: eight moderate, six high; build/development tooling remains a follow-up risk. Bundled UI input licenses are inventoried separately |
| Offline sidecar | Shared catalog: 51 methods; MessagePack transport/cancellation and valid speech WAV |
| Packaged Electron | All six panels launch without avatar/GPU acceleration; MessagePack present in ASAR; installed resource/model hashes and readable model modes verified |
| AppImage | Pinned modern runtime; custom launcher preserves sandbox policy; actual extract-and-run graphical smoke passed. Extraction-only success cannot satisfy this gate |
| Documents | Approve, restart, cited retrieval, durable delete, failed-approval retry, graph rollback exclusion, changed-embedding identity rejection |
| Code | Four initial entities/three relations; approved graph/vectors; paging, revisions, rollback, and graph/vector/artifact deletion verified using exported backend |
| GLM OCR | Actual `glm-ocr:latest` and bundled PDFium; generated PDF preserves 42.75 MPa and real page/figure evidence; coherent approval/restart/retrieval/delete; no temporary outputs left |
| Research | Knowledge: complete, 12 sources. Papers: partial, 74 sources/seven PDFs; engine/source failures visible. Exact PDF URL/SHA256 provenance and both source-linked reports survive restart |
| Chat/lifecycle | Packaged streaming, chat cancellation, research cancellation with no history entry, valid TTS, cleanup, clean shutdown |
| Default local chat | Actual `gemma4:e4b` inference: streamed output in 42,221 ms, with clean shutdown; public fixture only |
| Speech | Packaged Supertonic generates English/Turkish; Omnilingual CPU STT emits final transcripts from public generated 16 kHz PCM fixtures. English recognized “ala” for “Ayla”; this is not a real-microphone quality claim |
| Backup | Coordinated export/checksum verification; fresh database/vector-prefix/artifact-root restore; retrieval and literal source text preserved; original store intact |
| Source installer | Actual source build/install/status/uninstall under a path containing spaces; settings preserved. Offline build/install/uninstall also passed with nested dependency downloads disabled |
| Model bootstrap | Fresh-directory speech hydration from upstream; every model file verified by byte count/SHA256. Cached-archive offline hydration into another empty directory passed |
| Native Arch | Clean container source build, tests, system Electron 41 launch; pacman install/speech/remove checks preserve user state |
| AUR binary | Payload checksum-derived recipe; final `/usr/lib/ayla-ai-bin` layout verified through pacman install, graphical launch, speech and removal |
| Stalled-service startup | Controlled server never responds; backend starts in 20,009 ms with storage unavailable and local speech working |
| CI tooling | Builder UID/GID 1001 verified; pinned RustSec auditor builds with Rust 1.88 |
| Backend endurance | 30 minutes; 163 cycles; 28 speech runs. Post-warmup RSS 200,108 KiB → 217,824 KiB; peak 302,280 KiB. Descriptors stayed 18. No pending requests; clean shutdown |

The endurance host was an AMD Ryzen 5 7600 (six cores/12 threads), about 15 GiB
RAM, with swap enabled and parallel build workloads. Resident pages varied with
host paging. This is backend qualification, not desktop FPS or device acceptance.

## Build and payload identity

Portable: pinned Debian 12 image, Node 22.15.0, Rust 1.88.0, Electron 41.10.7,
Linux x86_64/glibc 2.36 baseline. Native Arch uses system Electron 41 and captures
its rolling build environment in `.BUILDINFO`. Containerized tests enable the
permissions needed for Chromium's nested sandbox; they do not pass `--no-sandbox`.

Final public candidates are under `release/github/`. `SHA256SUMS` covers the
AppImage, source installer, binary payload, native/source manifests, and copied
Arch packages. The generated AUR binary `.SRCINFO` refers to that exact payload.
The build is locked and isolated; byte-for-byte reproducibility is not claimed.

Installed notices inventory 448 Rust/JavaScript package inputs, with checksummed
notice files. It conservatively includes build-only/transitive inputs. Electron,
PDFium, native libraries and speech models retain separate notices. Three Rust
crates declare MIT in published metadata without shipping a standalone license
file (`adobe-cmap-parser`, `pdf-extract`, `type1-encoding-parser`); assembled
notices identify that fact. Their declarations need maintainer review alongside
GPL backend corresponding-source directions before redistribution. The user
confirmed redistribution permission for the OCR prototype.

Local Ollama setup verifies `glm-ocr:latest`, `qwen3-embedding:0.6b`, `gemma4:e4b`
and the existing authorized cloud model. Model bodies are not embedded in the
AppImage. Curated speech resources are embedded and hash-verified. Licensed
avatar/Cubism files and unreviewed Piper models are excluded.

## Publication gates and limits

- The unpublished base commit added a 365,352,120-byte speech-model Git blob.
  GitHub blocks ordinary Git files above 100 MiB. The downloader is prepared;
  the user authorized a clean release commit with model files retained locally
  and the original preserved on `backup/pre-linux-release-82cb395`. Push only
  the release branch; do not push that oversized local backup branch. No tag,
  push, or publication is performed by this task. [GitHub file-limit and unpublished-commit guidance](https://docs.github.com/en/repositories/working-with-files/managing-large-files/about-large-files-on-github).
- Review the prepared clean release commit, then publish the immutable signed release
  tag required by source installers/source AUR recipes. Import a trusted signing
  key; sign checksums. Future release URLs cannot be downloaded/tested before
  publication. AUR names were available through the public API during this run;
  recheck before submitting.
- Complete human OCR/research evidence/citation review, real Turkish/English
  microphone/device/permission checks, X11/Wayland and GPU/compositor acceptance,
  and the stated 3440×1440 FPS/frame-time target on reference hardware.
- Review migration against user-owned pre-release data and verify a coordinated
  backup before changing schemas. Legacy unscoped facts/episodes and content in
  retained snapshots are not automatically erased by source deletion.
- Generic source compatibility currently requires unique filenames; large graphs
  still use serialized snapshots. Incremental path-identity migration, a rich
  job/snapshot UI, and additional platforms/architectures remain follow-up work.

A verified local candidate does not remove these publication gates. Review
[the release procedure](../packaging/RELEASE.md) and [test plan](TEST_PLAN.md).
