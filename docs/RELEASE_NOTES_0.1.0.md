# Ayla 0.1.0 Linux candidate

The first Linux candidate uses Electron and the Rust backend. Tauri packages,
host code and runtime branches have been removed. It runs without the optional
licensed avatar and supports Linux x86_64 with glibc 2.36 or newer.

- AppImage, a versioned installer that builds from source, and native Arch/AUR
  source and binary recipes are prepared. Model weights are downloaded with
  pinned checksums rather than stored in source Git history.
- GLM OCR and PDFium remain the OCR pipeline. Approval, restart, cited retrieval
  and deletion share the durable knowledge lifecycle. Rendering, extraction,
  cache and download sizes are bounded.
- Research retains SearXNG and guarded browser/HTTP retrieval. It has independent
  model selection and cloud consent, durable history, visible partial failures,
  and exact URL/checksum provenance for downloaded PDFs.
- Documents, OCR and code analysis use the authoritative knowledge graph and
  source-linked vectors. Failed approvals are excluded from retrieval and can
  be retried. Embedding model changes require a deliberate migration.
- Coordinated backup/restore, service recovery, bounded startup, writer locks,
  private diagnostic logs and a filtered support export are included.
- Local Gemma chat, Qwen embeddings, GLM OCR, Supertonic speech synthesis and
  Omnilingual recognition are configured and verified. Cloud processing remains
  an explicit opt-in for each supported workflow.

Before upgrading existing data, create and verify a coordinated backup using
the [README instructions](../README.md). Application rollback does not reverse
schema migrations. Generic documents currently require unique filenames;
legacy unscoped facts and retained snapshots need a separate retention review.

This is a locally qualified candidate. Signing, publication, downloaded-artifact
acceptance, human OCR/research review, real microphone/device checks and
GPU/Wayland performance acceptance remain release gates. See
[qualification evidence](RELEASE_EVIDENCE.md) and the
[maintainer release procedure](../packaging/RELEASE.md).
