#!/usr/bin/env bash
set -euo pipefail
AYLA_INSTALLER_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
command -v node >/dev/null || { echo 'Install Node.js 22 or newer before building Ayla from source.' >&2; exit 1; }
exec node "$AYLA_INSTALLER_ROOT/scripts/install-source.mjs" "$@"
