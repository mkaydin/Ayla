#!/usr/bin/env bash
set -euo pipefail
AYLA_PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec node "$AYLA_PROJECT_ROOT/scripts/doctor.mjs" "$@"
