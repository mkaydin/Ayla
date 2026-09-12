#!/usr/bin/env bash
set -euo pipefail
export CARGO_BUILD_JOBS=4
mkdir -p src/ayla
for AYLA_ARCH_INPUT in Cargo.toml Cargo.lock package.json package-lock.json electron-builder.yml vite.config.js index.html LICENSE .env.example crates vendor src styles public/icon.png public/audio-capture-worklet.js electron scripts models tests packaging/licenses tts/supertonic3-int8; do
  mkdir -p "src/ayla/$(dirname -- "$AYLA_ARCH_INPUT")"
  cp -a "/source/$AYLA_ARCH_INPUT" "src/ayla/$AYLA_ARCH_INPUT"
done
cp /source/packaging/arch/ayla-ai/PKGBUILD .
mkdir -p /cache/target
ln -s /cache/target src/ayla/target
cd src/ayla
node scripts/bootstrap-native.mjs
node scripts/bootstrap-speech.mjs
npm ci --ignore-scripts
cargo fetch --locked --target x86_64-unknown-linux-gnu
cd /build
# Sources are the reviewed local candidate; the release tag is not published yet.
# Dependencies were installed into this clean image. Build/check remain offline.
makepkg --noextract --nodeps
namcap ./*.pkg.tar.zst
AYLA_ARCH_SMOKE_DIR="$(mktemp -d /tmp/ayla-arch-smoke.XXXXXX)"
AYLA_DATA_DIR="$AYLA_ARCH_SMOKE_DIR" AYLA_DISABLE_AVATAR=1 AYLA_RESOURCE_DIR=/build/src/payload AYLA_SIDECAR_BIN=/build/src/payload/bin/ayla-sidecar xvfb-run -a electron41 /build/src/payload/app.asar --smoke-test --disable-gpu
cp ./*.pkg.tar.zst /output/
