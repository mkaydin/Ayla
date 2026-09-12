import { writeFile } from 'node:fs/promises';
import path from 'node:path';

// The upstream launcher silently adds --no-sandbox when its namespace probe
// fails. Our launcher keeps sandbox policy intact and lets Chromium report
// unsupported hosts. appOutDir is copied over the generated AppImage launcher.
export default async function afterPack(context) {
  if (context.electronPlatformName !== 'linux') return;
  await writeFile(path.join(context.appOutDir, 'AppRun'), `#!/bin/sh
set -eu
APPDIR="\${APPDIR:-$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)}"
for argument in "$@"; do
  case "$argument" in --no-sandbox|--no-sandbox=*)
    echo 'Ayla requires Chromium sandboxing. Use a supported host or the native Arch package.' >&2
    exit 64;;
  esac
done
export XDG_DATA_DIRS="$APPDIR/usr/share:\${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
exec "$APPDIR/ayla" "$@"
`, { mode: 0o755 });
}
