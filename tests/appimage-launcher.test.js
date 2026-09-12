import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync, chmodSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import afterPack from '../electron/after-pack.mjs';

test('AppImage launcher preserves arguments and refuses sandbox disabling', async () => {
  const root = mkdtempSync(path.join(tmpdir(), 'ayla launcher '));
  try {
    await afterPack({ electronPlatformName: 'linux', appOutDir: root });
    writeFileSync(path.join(root, 'ayla'), '#!/bin/sh\nprintf "%s\\n" "$@"\n');
    chmodSync(path.join(root, 'ayla'), 0o755);
    const env = { ...process.env }; delete env.APPDIR;
    const launch = spawnSync(path.join(root, 'AppRun'), ['argument with spaces', '--smoke-test'], { env, encoding: 'utf8' });
    assert.equal(launch.status, 0);
    assert.equal(launch.stdout, 'argument with spaces\n--smoke-test\n');
    assert.equal(spawnSync(path.join(root, 'AppRun'), ['--no-sandbox'], { env }).status, 64);
    assert.ok(!readFileSync(path.join(root, 'AppRun'), 'utf8').includes('unshare'));
  } finally { rmSync(root, { recursive: true, force: true }); }
});
