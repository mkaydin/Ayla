# Model Licenses

## Supertonic 3 INT8

- Model files: `tts/supertonic3-int8/*`
- Model license: `tts/supertonic3-int8/MODEL_LICENSE`
- License: BigScience Open RAIL-M
- Model source: <https://huggingface.co/Supertone/supertonic-3>
- Sherpa-ONNX conversion archive: <https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models>
- Upstream model card: `tts/supertonic3-int8/MODEL_CARD.md`

The Open RAIL-M use restrictions apply to the model and derivatives. Any Ayla
distribution containing these weights must include the complete model license
and notify recipients that its use restrictions apply. The MIT file shipped at
the root of the sherpa conversion archive is retained separately as
`UPSTREAM_REPOSITORY_LICENSE`; it must not be presented as the model license.

The current Ayla integration uses the bundled F1 voice style and does not claim
that generated speech is a real person's voice.

## Omnilingual ASR 300M CTC INT8

- Model directory: `models/stt/sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-int8-2025-11-12`
- License: Apache License 2.0, included as `LICENSE`
- Provider: CPU

## Silero VAD

- Model file: `models/stt/silero_vad.onnx`
- License: MIT, included as `models/stt/SILERO_VAD_LICENSE`
- Source: <https://github.com/snakers4/silero-vad>

## Native runtime and first-release exclusions

Ayla source remains MIT; Sherpa-ONNX bindings are Apache-2.0, and PDFium's
archive includes its BSD license and third-party notices. Packaging includes
PDFium notices plus Debian copyright notices for copied native libraries.

The selected Sherpa-ONNX static archive links eSpeak-NG into the sidecar even
when Supertonic is selected. Symbol inspection confirms this dependency.
eSpeak-NG is [GPLv3](https://github.com/espeak-ng/espeak-ng/blob/1.52.0/COPYING);
the complete license is included at `packaging/licenses/ESPEAK_NG_GPL3`.
Maintainers must satisfy corresponding-source and notice requirements for the
combined binary before publication; the MIT app-source declaration alone is
not a complete binary license statement. Native upstream source/build inputs
are identified by `vendor/sherpa-onnx/UPSTREAM.md` and the pinned 1.13.4 archive.
The release checklist keeps final corresponding-source verification explicit.

Piper Amy and its espeak data are excluded from first-release payloads. Its
model card refers to an external dataset license without a clear model-specific
license grant. Development fallback support is retained for separately supplied
assets; no permission for those weights is assumed.

Ollama weights are downloaded separately by the user and retain their upstream
model licenses. Live2D avatars and Cubism Core are optional user-supplied
assets, excluded from all packages. OCR prototype redistribution permission
was explicitly confirmed by the maintainer on 2026-10-13; see the provenance
record and retain the underlying permission with release records.

## Dependency notices and model acquisition

Packaging generates `licenses/DEPENDENCY_LICENSES.json` plus checksummed notices
for locked Rust inputs and the bundled JavaScript dependency graph. The inventory
includes conservative build/transitive inputs and does not replace native or
model notices. Three published MIT crates omit standalone license files; their
assembled notices explicitly preserve that distinction for maintainer review.

`scripts/bootstrap-speech.mjs` verifies pinned upstream archive SHA256 values and
all declared model file sizes/digests. It streams individual archive members
without extracting archive paths or links. Offline setup uses verified caches.
Large weights must be omitted from ordinary Git history or tracked with LFS.
