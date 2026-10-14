<p align="center"><img src="images/ayla-symbol.svg" width="150" alt="Ayla"></p>
<h1 align="center">Ayla</h1>

Ayla is a Linux desktop AI coworker with streaming chat, persistent knowledge,
GLM OCR, research, code analysis, and offline Turkish/English speech. Electron
hosts the interface; one Rust sidecar runs the backend and inference services.
The optional Live2D avatar is supplied separately. Chat and tools work without it.

The first release targets **Linux x86_64**: AppImage, a user installer that builds
from source, and native Arch/AUR recipes. The portable build targets glibc 2.36
or newer. Cross-distribution desktop acceptance remains a release gate; see
[the production plan](docs/PRODUCTION_READINESS_PLAN.md) and
[release procedure](packaging/RELEASE.md) for current evidence.

<p align="center"><img src="images/Screenshot_20260805_095628.png" alt="Ayla chat and optional avatar" width="100%"></p>

## Features

- Ollama chat with streaming, cancellation, persistent sessions, Markdown,
  syntax highlighting, sanitized HTML, and KaTeX mathematics.
- Supertonic 3 INT8 F1 speech in Turkish and English; Omnilingual 300M INT8
  dictation with Silero VAD. Models and their licenses are bundled and hashed.
- In-process PDFium geometry and `glm-ocr:latest` vision OCR, bounded workers,
  model-version-aware resume cache, page citations, deterministic knowledge
  extraction, and optional semantic assembly with explicit cloud consent.
- General-knowledge gathering and scientific-paper research with SearXNG,
  bounded public-URL fetching, source-linked reports, and visible partial results.
  Guarded Lightpanda DOM extraction falls back to HTTP extraction when the
  required isolation controls are unavailable. Remote page scripts are disabled.
- Tree-sitter code analysis, optional semantic analysis, reviewable artifacts,
  revision comparison, approved graph/vector search, and run removal.
- One authoritative UID graph with bounded UI pagination. Document approval
  records a durable operation before changing storage; failed imports are
  excluded from retrieval and can be retried from Knowledge Base → Files.

## Local prerequisites

Install Node >=22.12, Rust >=1.88, npm, Git, a C/C++ toolchain, pkg-config,
OpenSSL/xz/bzip2 development libraries, and Ollama. A graphical Linux desktop
and Electron's native GTK/NSS/audio/GBM libraries are required to launch.
Docker Compose is an optional way to provide local persistence/search services;
existing compatible services can instead be configured by endpoint.

```bash
npm ci
npm run native:setup
npm run models:setup
npm run services:setup -- --start
npm run electron:dev
```

Model setup downloads missing `glm-ocr:latest`, `qwen3-embedding:0.6b`, and
`gemma4:e4b`. Ollama must already be running. `npm run models:setup -- --cloud`
also prepares the optional `nemotron-3-super:cloud` tag; cloud authorization and
application consent are separate. LLM weights are not bundled in the AppImage.
Speech weights are downloaded and verified from pinned upstream archives during setup.
The native bootstrap verifies fixed Sherpa-ONNX/PDFium archive checksums.

`services:setup` generates missing secrets and creates storage owned by your
user. It preserves an existing `.env`. The Compose stack binds ports to loopback:
SurrealDB 8000, Qdrant 6333/6334, SearXNG 8888, and Lightpanda 9222. It never
starts Ollama or downloads LLMs implicitly.

Containers are started explicitly after the project disk is mounted; automatic
restart is disabled. Run `npm run services:setup -- --start` after reboot or
remounting the disk. Compose refuses missing bind-mount paths instead of creating
empty replacement storage. To stop services while retaining data, run
`docker compose stop`.

## Install from source

From a reviewed checkout:

```bash
./install.sh --source-dir "$PWD" --models
```

From a published, signed release tag:

```bash
./install.sh --ref v0.1.0 --models
```

Import the maintainer's signing public key before tag verification. An immutable
full commit can be selected with `--commit`; `--allow-unsigned` is an explicit
development override. `--offline --source-dir ...` requires cached npm/Cargo
packages and verified native archives. The installer runs checks, builds the
application, and activates a versioned user installation at `~/.local/opt/ayla`.
It creates `~/.local/bin/ayla` and a desktop entry. Add `~/.local/bin` to PATH.
It refuses to replace an unrelated launcher.

```bash
./install.sh --status
./install.sh --rollback
./install.sh --uninstall
```

Uninstall preserves configuration, knowledge, artifacts, and Ollama models.
Application rollback preserves data and does not reverse schema changes. Close
Ayla and create a verified snapshot before upgrading storage schemas.

## AppImage and Arch

Download the AppImage and `SHA256SUMS` from the reviewed GitHub release. Verify
the checksums/signature, make the file executable, and run it. The runtime
requires a working Chromium sandbox; it is never launched with `--no-sandbox`.
If FUSE is unavailable, use the AppImage's `--appimage-extract-and-run` mode.

[Arch/AUR recipes](packaging/arch/README.md) provide source and binary packages.
The source package uses system Electron 41. Packages do not start services or
pull Ollama models in installation hooks. AUR submission is a maintainer step.

Speech weights are acquired by `npm run native:setup` from the pinned upstream
archives in `models/model-manifest.json`; archive and individual file digests
are verified. They belong in the local model cache rather than ordinary Git
history. Offline source installation can hydrate them from previously cached
verified archives. AppImage and Arch binary payloads include curated speech
resources, so runtime speech needs no model download.

## Settings and privacy

Development reads the checkout `.env`. Installed builds read
`${XDG_CONFIG_HOME:-~/.config}/ayla/settings.env`; copy `.env.example` there and
set credentials/endpoints. `AYLA_CONFIG_FILE` overrides the configuration path,
`AYLA_DATA_DIR` the data directory, and `AYLA_RESOURCE_DIR` installed resources.
An explicitly selected configuration never falls back to another `.env`.
`AYLA_DATA_DIR` and `AYLA_RESOURCE_DIR` are launch-environment overrides; changing
the settings-file location does not move the application data directory.

`AYLA_LLM_MODEL` selects chat; `AYLA_RESEARCH_MODEL` independently selects
research and falls back to chat when unset. OCR, embeddings, and semantic code
analysis have their own model settings. OCR/code cloud consent defaults to
false. Cloud research requires `AYLA_RESEARCH_ALLOW_CLOUD=true` and sends the
topic and selected source evidence to the configured model. Choose a local
research model for local processing. Existing cloud model selection alone does
not enable this new research opt-in.

One backend writer owns each data directory. Run only one application against
an associated database/vector namespace. Storage-dependent approval is blocked
while persistence is unavailable; an idle backend retries startup without
replacing the window. Failed/deleting sources remain excluded from retrieval.
Duplicate document names with different paths are rejected; rename the incoming
source or explicitly remove the existing one. This release does not silently
merge identically named documents from different directories.

The configured embedding model, digest (when Ollama is available), and vector
dimensions are recorded. A changed identity blocks storage initialization;
restore the recorded model, or choose a fresh database/vector prefix and
explicitly reapprove source inputs. Changing a model never reuses incompatible
vectors. Pre-release databases without this metadata adopt the configured
identity once; verify their existing model and back them up before upgrading.
Document lists load metadata only. Full text is fetched on demand, with a
128 MiB memory cache and 32 MiB per-document limit.

OCR inputs are limited to 100 MiB/5,000 pages, rasters to 16 million pixels,
rendered temporary images to 512 MiB, and retained resume caches to 2 GiB.
Research downloads have byte limits; concurrent workers belong to their request
and stop on cancellation. Private research stage records live under
`ayla/research/jobs`; interrupted jobs are marked on backend startup.
Downloads and completed reports remain until explicitly removed; monitor free
disk space. A research request downloads at most ten PDFs of at most 100 MiB each.

Runtime support logs rotate at 1 MiB with two previous files. They record only
allowlisted lifecycle/version fields. Export them with
`node scripts/support-bundle.mjs /path/to/support.json` and review before sharing.
The exporter excludes backend stderr, settings, prompts, documents, and audio.

## Backup and verification

Graph-only rollback does not restore documents, vectors, or approval state.
Deleted/failed sources stay excluded after graph rollback. Automatic graph
backups retain the latest 25 snapshots and may contain deleted content until
pruned. Coordinated snapshots are retained until you remove them.

Close Ayla before exporting. Snapshots contain private knowledge and should
stay local. Only restore trusted snapshots; checksums detect corruption rather
than authenticate an unknown SQL export.

`npm run migration:review` provides a read-only aggregate review of document and
graph counts, pending operations, retained graph snapshots, and embedding identity.
Select development settings explicitly with
`npm run migration:review -- --config .env`. This review does not replace a verified coordinated backup.

```bash
node scripts/knowledge-backup.mjs export --output /path/to/private-backups
node scripts/knowledge-backup.mjs verify --snapshot /path/to/snapshot
node scripts/knowledge-backup.mjs restore --snapshot /path/to/snapshot \
  --database restored_memory --collection-prefix restored_ \
  --data-dir /path/to/new-ayla-data
```

Restore targets must be fresh and different from active storage. The command
restores structured storage, vectors, and owned artifacts together; it leaves
configuration switching to you after checking retrieval. For installed-only
backup tooling set `AYLA_SIDECAR_BIN` to that package's backend binary.

```bash
npm run check
cargo test --workspace --locked
npm run sidecar:diagnostic
npm run doctor
npm run release:linux
```

See [the test plan](docs/TEST_PLAN.md) for isolated integration checks. The
portable release builder uses pinned Debian/Node/Rust images, verifies native
and speech assets, inspects the ASAR, and launches an avatar-free installed
renderer under Xvfb. Build artifacts are staged under `release/github`.

## License and optional assets

Ayla source is MIT. Bundled models retain their own terms, including the
Supertonic Open RAIL-M use restrictions. The statically linked speech runtime
includes eSpeak-NG under GPLv3; binary/source distribution obligations and
notices are recorded in [the license inventory](docs/model-licenses.md).
The older Piper model is not included in first-release packages pending a
model-specific license review. Development can still use separately supplied
Piper resources. Live2D/Cubism/avatar files are optional, never bundled or
silently downloaded; supply lawfully obtained assets and the local Cubism core
in `AYLA_LIVE2D_MODEL_DIR` to enable the avatar.
