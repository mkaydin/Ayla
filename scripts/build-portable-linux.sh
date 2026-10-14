#!/usr/bin/env bash
set -euo pipefail
AYLA_BUILD_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
AYLA_RELEASE_OUTPUT="$(realpath -m -- "${AYLA_RELEASE_OUTPUT:-$AYLA_BUILD_ROOT/release/github}")"
mkdir -p "$AYLA_RELEASE_OUTPUT"
AYLA_BUILD_USER_ID="$(id -u)"
AYLA_BUILD_GROUP_ID="$(id -g)"
if [ "$AYLA_BUILD_USER_ID" = 0 ]; then echo 'Run the release build as a normal user.' >&2; exit 1; fi
AYLA_RELEASE_CACHE=ayla-release-build-cache
if [ "$AYLA_BUILD_USER_ID" != 1000 ]; then AYLA_RELEASE_CACHE="$AYLA_RELEASE_CACHE-uid$AYLA_BUILD_USER_ID"; fi
docker build --build-arg "AYLA_BUILD_UID=$AYLA_BUILD_USER_ID" --build-arg "AYLA_BUILD_GID=$AYLA_BUILD_GROUP_ID" -f "$AYLA_BUILD_ROOT/packaging/linux/Dockerfile" -t ayla-release-builder "$AYLA_BUILD_ROOT/packaging/linux"
AYLA_SOURCE_REVISION="$(git -C "$AYLA_BUILD_ROOT" rev-parse HEAD)"
docker run --env "AYLA_SOURCE_REVISION=$AYLA_SOURCE_REVISION" --rm --cap-add SYS_ADMIN --security-opt seccomp=unconfined --mount "type=volume,src=$AYLA_RELEASE_CACHE,dst=/cache" --mount "type=bind,src=$AYLA_BUILD_ROOT,dst=/source,readonly" --mount "type=bind,src=$AYLA_RELEASE_OUTPUT,dst=/output" ayla-release-builder
