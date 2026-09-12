#!/usr/bin/env bash
set -euo pipefail
AYLA_ARCH_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$AYLA_ARCH_ROOT/release/arch"
AYLA_ARCH_USER_ID="$(id -u)"
AYLA_ARCH_GROUP_ID="$(id -g)"
if [ "$AYLA_ARCH_USER_ID" = 0 ]; then echo 'Run the release build as a normal user.' >&2; exit 1; fi
AYLA_ARCH_CACHE=ayla-arch-build-cache
if [ "$AYLA_ARCH_USER_ID" != 1000 ]; then AYLA_ARCH_CACHE="$AYLA_ARCH_CACHE-uid$AYLA_ARCH_USER_ID"; fi
docker build --build-arg "AYLA_BUILD_UID=$AYLA_ARCH_USER_ID" --build-arg "AYLA_BUILD_GID=$AYLA_ARCH_GROUP_ID" -f "$AYLA_ARCH_ROOT/packaging/arch/Dockerfile" -t ayla-arch-builder "$AYLA_ARCH_ROOT/packaging/arch"
docker run --rm --cap-add SYS_ADMIN --security-opt seccomp=unconfined --mount "type=volume,src=$AYLA_ARCH_CACHE,dst=/cache" --mount "type=bind,src=$AYLA_ARCH_ROOT,dst=/source,readonly" --mount "type=bind,src=$AYLA_ARCH_ROOT/release/arch,dst=/output" ayla-arch-builder
