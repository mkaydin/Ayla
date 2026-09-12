> Historical design record. Runtime and setup instructions are superseded by [README](../README.md) and the [production readiness plan](PRODUCTION_READINESS_PLAN.md).

# Mk6 Live2D Integration Plan

The Mk6 model replaces the legacy DemonGirl runtime. Its licensed source files
remain a local installation; the application code, generated branding assets,
and model adapter are the only parts tracked in Git.

## Phase 1 — Asset boundary and model profile

- [x] Ignore `Mk6_1.0/`, `public/Mk6/`, and `.mk6-cache/`.
- [x] Keep Mk6 source assets out of Vite and Electron packages.
- [x] Catalog all 21 expressions, 3 motions, and Mk6 parameter IDs.
- [x] Complete the sparse model manifest at runtime without editing the source.
- [x] Serve local assets through a path-safe Vite/Electron mount.
- [x] Provide a read-only packaged-Tauri fallback for local assets.

## Phase 2 — Native Cubism runtime

- [x] Replace the procedural animation controller with native Cubism motions.
- [x] Load authored expression parameters at runtime without packaging them.
- [x] Replace Cubism's single expression slot with a fade-aware layered mixer.
- [x] Preserve Mk6 physics and authored idle, tracking-lost, and sleep motions.
- [x] Prevent mouse look, blink, and mouth layers from fighting full-body motions.
- [x] Keep audio-timed lip sync on `ParamMouthOpenY` and `ParamJawOpen`.

## Phase 3 — Application integration

- [x] Add every authored motion and expression to the Status panel selector.
- [x] Show active expression layers and allow each compatible layer to be toggled.
- [x] Map LLM expression tags to Mk6 expressions.
- [x] Map background task start/success/error/cancel states to Mk6 expressions.
- [x] Wake the model from sleep before TTS playback.
- [x] Validate combined Outfit + Tablet + Pen + Happy rendering in Electron.
- [ ] Visually tune per-expression fade timing after extended in-app testing.

## Phase 3B — Video-matched expression composition

- [x] Keep emotions mutually exclusive while preserving props and appearance.
- [x] Allow Tablet + Pen and outfit + eye-mask combinations.
- [x] Prevent Controller/Tracking from conflicting with occupied arm slots.
- [x] Keep indicators, blush, and interaction effects independently toggleable.
- [x] Cross-fade conflicting layers and remove inactive layers after fade-out.
- [x] Preserve LLM and process expressions without clearing user-selected props.

## Phase 4 — Legacy removal

- [x] Remove the tracked DemonGirl model and public symlink.
- [x] Remove its procedural animation catalog, controller, parameter profile, and tests.
- [x] Remove active source/config references to the old model.
- [x] Update setup, health-check, and test documentation for Mk6.

## Phase 5 — Theme and branding

- [x] Shift the application palette from purple to navy, blue, cyan, and teal.
- [x] Apply the blue palette to panels, the Pixi background, and KB graph communities.
- [x] Crop `icon_STD.png` into a neutral Alice application icon.
- [x] Regenerate Electron, Tauri, Windows, Linux, and macOS icon resources.
- [x] Update application identifiers and visible titles.

## Phase 6 — Validation

- [x] Run JavaScript unit tests and Rust path-security tests.
- [x] Build the Vite application and Electron package.
- [x] Verify no Mk6 model, expression, motion, or `.moc3` files enter the package.
- [x] Smoke-test the packaged Electron application on KDE/Wayland.
- [ ] Complete a manual visual pass through all 21 expressions and 3 motions.

## Phase 7 — Semantic behavior system

- [x] Add a central animation coordinator with state priority and owned layer leases.
- [x] Keep manual accessories restorable after automatic Chat and process poses.
- [x] Add cursor-led head/body tracking, breathing, and procedural leg weight shifts.
- [x] Use Tablet while Chat is generating, streaming, or speaking through TTS.
- [x] Use Tablet + Pen with non-repeating writing movements for Search, OCR,
      document extraction, KB ingestion, and graph analysis.
- [x] Block sleep while a process, Chat response, or TTS playback is active.
- [x] Enter authored Sleep after five minutes of inactivity and restrict its
      overlays to compatible appearance layers and Eyemask.
- [x] Enter tracking-lost when a globally required service, or a service needed
      by the active operation, is confirmed offline.
- [x] Apply transient LLM and process-result expressions without exposing their
      control tags in rendered Chat or TTS text.
- [x] Show the current semantic behavior in the Status panel.
- [ ] Visually tune body/leg ranges and pen stroke placement on the installed
      Mk6 model at full application scale.

## Local model locations

Development defaults to `<project>/Mk6_1.0`. `ALICE_LIVE2D_MODEL_DIR` overrides
that path. Packaged Electron and Tauri builds also look for `Mk6_1.0` beside the
executable or in the application's data directory.
