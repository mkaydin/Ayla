# Native runtime notices and corresponding source

The Ayla backend includes eSpeak-NG under GPLv3. Recipients may copy, modify,
and redistribute the combined backend under GPLv3's terms. There is no warranty.
Ayla application sources retain their MIT notices. See ESPEAK_NG_GPL3 for the
complete license. The independent Electron runtime and model files retain their
own licenses and notices.

Source/download directions for the Linux x86_64 native backend:

- Ayla backend, wrappers, changes, installation and build scripts: the source
  archive/tag accompanying this exact GitHub release at
  https://github.com/mkaydin/Ayla. Cargo.lock identifies Rust dependency sources.
- Sherpa-ONNX native source and build scripts, v1.13.4:
  https://github.com/k2-fsa/sherpa-onnx/tree/v1.13.4
- eSpeak-NG fork source, revision ed530aa113046142eb5115cf2fc9157854d0ffe1:
  https://github.com/csukuangfj/espeak-ng/archive/ed530aa113046142eb5115cf2fc9157854d0ffe1.zip
  SHA256 e4e262cbe34f7fe21f91f1ba3397f2728e1f30eafbae7853f2b753a9ed13f0dd
- ONNX Runtime source, v1.27.0:
  https://github.com/microsoft/onnxruntime/tree/v1.27.0
  Static-library build scripts:
  https://github.com/csukuangfj/onnxruntime-libs/tree/v1.27.0
- Remaining native dependency source revisions and archive hashes are declared
  in the Sherpa-ONNX v1.13.4 cmake files. These include kaldifst/openfst,
  kaldi-decoder, kaldi-native-fbank, KissFFT, libucd and ssentencepiece.

The native archive used by Ayla has SHA256
98b0e31996426f6e78244dbce1955548f2c64e8f01c4be75b85af7cdaa2e8d5c.
Binding import revision and Ayla modifications are documented in
vendor/sherpa-onnx/UPSTREAM.md in the accompanying source tree.

To rebuild native libraries, use the corresponding Sherpa-ONNX Linux x86_64
static-library release build scripts with those pinned cmake inputs. Ayla's sys
wrapper accepts SHERPA_ONNX_LIB_DIR for a locally rebuilt library directory;
see its build.rs. Then run the matching Ayla source build/installer. The normal
bootstrap downloads the checksum-verified prebuilt archive instead.

Maintainers must keep these equivalent source download directions available
alongside the binary release and verify all upstream links and terms before
publication. PDFium and copied system libraries have separate accompanying
copyright/license notices. Model restrictions are in the model license files.
