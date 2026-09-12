#!/usr/bin/env bash
set -euo pipefail
# /source is mounted read-only; private configuration, databases, avatars,
# local node_modules and target trees are never copied into the build.
for AYLA_BUILD_ITEM in Cargo.toml Cargo.lock package.json package-lock.json electron-builder.yml vite.config.js index.html install.sh LICENSE .env.example crates vendor src styles public/icon.png public/audio-capture-worklet.js electron scripts models tests packaging/licenses tts/supertonic3-int8; do
  mkdir -p "/work/$(dirname -- "$AYLA_BUILD_ITEM")"
  cp -a "/source/$AYLA_BUILD_ITEM" "/work/$AYLA_BUILD_ITEM"
done
mkdir -p /cache/target
ln -s /cache/target /work/target
node scripts/bootstrap-native.mjs
node scripts/bootstrap-speech.mjs
export PDFIUM_LIB_PATH=/work/target/native/pdfium/lib/libpdfium.so
npm ci
npm run check
cargo test --workspace --locked
cargo build --locked -p ayla-sidecar
npm run sidecar:diagnostic
npm run electron:build
AYLA_SMOKE_ROOT="$(mktemp -d /tmp/ayla-package-smoke.XXXXXX)"
AYLA_DATA_DIR="$AYLA_SMOKE_ROOT" AYLA_DISABLE_AVATAR=1 xvfb-run -a release/electron/linux-unpacked/ayla --smoke-test --disable-gpu
node electron/smoke-appimage.mjs
cp release/electron/*.AppImage /output/
cp .electron-resources/native-manifest.json /output/
mkdir -p /output/source-installer/scripts
cp /work/install.sh /output/source-installer/install.sh
cp /work/scripts/install-source.mjs /output/source-installer/scripts/install-source.mjs
cp /work/LICENSE /output/source-installer/LICENSE
chmod 755 /output/source-installer/install.sh
(cd /output && tar -czf "Ayla-source-installer.tar.gz" source-installer)
rm -rf /output/source-installer
node scripts/release-artifacts.mjs /output
