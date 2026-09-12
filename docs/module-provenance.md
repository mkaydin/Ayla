# Integrated Module Provenance

All production integrations are maintained inside the Ayla repository. The
source repositories below are import records only: Ayla manifests, runtime
defaults, tests, and packages must never refer to their local filesystem paths.

| Module | Imported revision | Ayla destination | Import status | License status |
| :--- | :--- | :--- | :--- | :--- |
| STT prototype (`ayla_stt_test_module`) | `f58e86d` | `crates/ayla-stt`, `models/stt` | Integrated CPU/renderer-capture slice | Omnilingual: Apache-2.0; Silero VAD: MIT |
| TTS prototype (`ayla_tts_test_module`) | `6139e55` | `crates/ayla-tts`, `tts/supertonic3-int8` | Integrated first slice | Model: BigScience Open RAIL-M; prototype/sample code: MIT |
| OCR prototype (`glm_ocr_tests`) | `b28f605` | `crates/ayla-ocr` | Local extraction behavior adapted in place; optional assembly remains | Maintainer confirmed redistribution permission on 2026-10-13; retain permission record |
| Code analyzer (`kb_ready_code_doc`) | `7452645` | `crates/ayla-code-analysis` | Internal structured-analysis crate integrated; approved Graph-v2 and vector persistence integrated | Source manifest declares MIT OR Apache-2.0 |

## TTS adaptation record

The Python runtime was not copied. Ayla reimplemented the tested behavior in
Rust using its already-vendored sherpa-onnx 1.13.4 bindings:

- persistent Supertonic 3 INT8 engine using bundled F1 voice style;
- CPU provider, four threads by default, eight generation steps, speed 1.0;
- validated per-request Turkish `lang=tr` and English `lang=en` metadata;
- whitespace normalization followed by Unicode NFKD normalization;
- model completeness checks and PCM16 mono WAV output.

The source model files were copied byte-for-byte into
`tts/supertonic3-int8`. Their expected sizes and SHA-256 digests are recorded
in `models/model-manifest.json`. `MODEL_LICENSE` was obtained from the
authoritative Supertone model repository because the release archive's root
`LICENSE` describes sample code rather than the accompanying weights.

The prototype's untracked demo video and generated audio outputs were not
imported.

## STT adaptation record

The Python, NumPy, sounddevice, and cloud-repair runtime was not copied. Ayla
reimplemented Omnilingual ASR, Silero VAD segmentation, partial/final events,
bounded PCM input, and session lifecycle in Rust. Microphone capture uses the
sandboxed renderer's browser media API and sends explicitly bounded 16 kHz
PCM16 chunks to the same Ayla sidecar; no extra executable is launched.

The CPU INT8 Omnilingual model and Silero VAD files were copied byte-for-byte
into `models/stt`. The FP32, Whisper, and Qwen test models and the prototype's
local recording were deliberately not imported. Transcript repair remains off,
so microphone text cannot be sent to a cloud model by this integration.

## Code analyzer adaptation record

The library source and its generation tests were copied into
`crates/ayla-code-analysis`; generated Markdown outputs and reports were not
imported. Ayla's version adds structured entities/relations to the public
result, content-based path-independent repository fingerprints, serializable
diagnostics, cloud semantic analysis off by default, HTTPS-only remote Git with
explicit opt-in, a separate SSH opt-in, branch validation, and disabled Git
credential prompts.

Approved analyzer results now write their stable UIDs and source evidence to
the authoritative Graph-v2 store and dedicated vector collections. Chat
retrieval includes approved code with citations. Import, paging, rollback,
deletion and coordinated backup/restore have been verified with isolated
stores; existing user-owned data still requires migration review.

## OCR adaptation record

The prototype CLI and its Poppler processes were not imported. Ayla adapted
the page-layout, scientific Markdown prompt, quality-gate, and deterministic
assembly behavior into its existing Rust 2021 `ayla-ocr` crate. PDFium now
provides both page images and embedded-text geometry; scanned pages without a
text layer use an explicit one-column fallback instead of failing.

GLM runs through Ayla's configured local Ollama endpoint. Page concurrency is
bounded, transport and quality retries are finite, page order is restored
before merge, and cancellation drops the owned task set and temporary PDFium
workspace. Repetition loops, outer fences/entities, and echoed prompt text are
cleaned with page-level warnings. Final Markdown and layout JSON are written
atomically under Ayla's own application-data artifact directory.

Advanced processing remains inside the Ayla crate: successful page results are
stored in a private content-and-options-keyed resume cache, while optional
semantic assembly accepts only a loopback Ollama endpoint. Local models are the
safe default; cloud-tagged models require explicit consent and receive only the
bounded page evidence needed for assembly. Its JSON contract adds cited concepts,
facts, summaries, and cross-page relationships without replacing deterministic page evidence. Generated golden
PDFs cover the supported layout shapes, and a 5,000-page assembly test guards
runtime and retained-output growth.

No PDFs, generated output, binary, Cargo target data, or dependency on the
prototype repository was imported. The prototype repository has no declared public license. On 2026-10-13 the
maintainer explicitly confirmed they have redistribution permission for its
code. This permission confirmation clears the previously recorded provenance
blocker; retain the underlying permission record with the release records.
It does not grant third parties a license to the separate prototype repository.
