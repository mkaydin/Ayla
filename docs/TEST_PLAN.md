# Linux release verification

Use the final Electron/Rust checkout. Historical migration plans are design
records and contain obsolete setup instructions.

## Automated checks

```bash
npm ci
npm run native:setup
npm run check
PDFIUM_LIB_PATH="$PWD/target/native/pdfium/lib/libpdfium.so" cargo test --workspace --locked
npm run sidecar:diagnostic
```

The command diagnostic reads the shared Rust command catalog. It checks offline
errors, protocol/binary transport, cancellation, and speech without touching
user storage. Packaging inspection verifies production imports in the ASAR,
backend protocol, native resources, and model hashes.

## Isolated integrations

```bash
docker compose -p ayla-release-tests -f compose.test.yml up -d
npm run sidecar:diagnostic:live
npm run sidecar:diagnostic:documents
npm run sidecar:diagnostic:backup
npm run sidecar:diagnostic:ocr
npm run sidecar:diagnostic:code:live
npm run sidecar:diagnostic:features
npm run sidecar:diagnostic:research
npm run sidecar:diagnostic:soak -- 30
node electron/sidecar-startup-diagnostic.mjs
# Mono signed-16-bit little-endian PCM at 16 kHz:
npm run sidecar:diagnostic:stt -- /path/to/public-speech.pcm
```

Every diagnostic uses a unique database, vector prefix, and temporary data root.
The test Compose stack uses in-memory/tmpfs storage and different ports from
user services. Model endpoints are inherited. OCR uses generated public
fixture content; cloud-enabled checks may call the locally authorized cloud
model. Do not replace diagnostic endpoints with production storage.

```bash
docker compose -p ayla-release-tests -f compose.test.yml down
```

This removes only disposable test storage. Qdrant needs the configured 65,536
file-descriptor limit for repeated isolated collection creation.

Document/OCR checks must approve, restart, retrieve cited evidence, delete,
restart, and verify structured/text/vector/graph removal. Inject a missing
embedding endpoint and confirm failed approval remains durable, excluded, and
retryable. Backup checks export, verify hashes, restore into fresh storage,
verify retrieval, then confirm the original store remains intact.
Document checks also reject a changed embedding identity and keep deleted
sources excluded after a graph-only rollback.

Code checks cover snippet/directory analysis, preview exclusion, approved
retrieval, native UID graph paging, revision replacement, rollback, and deletion.
The GLM fixture checks numeric units, figure evidence, real page citations,
model digest recording, structured import, and temporary-workspace cleanup.
These automated fixtures do not replace a reviewed OCR quality corpus.

## Package and installer gates

Run `npm run release:linux`. The container builds from explicit inputs without
developer configuration, ignored avatar assets, node_modules, or target data.
It tests Node 22/Rust 1.88, runs archive checks, and starts the installed app
under Xvfb with Chromium sandboxing enabled. Extra container permissions permit
the nested sandbox for this test; production sandbox settings stay enabled.

Verify exported AppImage checksums. Test AppImage/FUSE and extract-and-run on
Arch and Debian 12 or a compatible supported desktop. Check X11 and Wayland,
missing avatar, absent services, later recovery, offline Ollama, denied mic,
active-work shutdown, repeated starts, and paths with spaces.

Build the source installer in an isolated user directory. Verify first install,
upgrade, application rollback, launcher/desktop integration, and uninstall.
Confirm existing settings/data survive. Build both Arch recipes with `makepkg`
and, before AUR submission, in a clean chroot with `namcap` inspection.

## Human acceptance still required

Record hardware, GPU/driver, compositor, application/native/model digests,
commands, timestamps, and observed outcomes in the release evidence file.

- Reviewed OCR corpus: scans, rotation, columns, multilingual text, tables,
  equations, malformed/encrypted/oversized/long inputs; assess factual evidence,
  units, reading order, and page attribution against frozen tolerances.
- Both research modes: source identity and citation grounding, contradictory
  findings, PDF provenance, partial failures, and approval/deletion.
- Real microphone and Turkish/English speech quality, permissions, noise,
  interruptions, and device changes.
- A 30-minute mixed-workload soak; bounded queues/listeners/RAM/processes after
  warmup, no orphan sidecar, responsive cancellation and shutdown.
- Target display performance: record idle FPS and p95 frame time on the stated
  reference hardware; do not infer graphical quality from an Xvfb smoke test.

Unexecuted acceptance checks remain explicit release gates. No script publishes
artifacts or assumes a signing key or permission decision.
