import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, readlink, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { activatePayload } from '../scripts/install-source.mjs';

test('source installer preserves relative links, switches versions atomically, and records rollback', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'ayla-installer-test-'));
  try {
    const payload = path.join(directory, 'payload');
    await mkdir(path.join(payload, 'resources', 'bin'), { recursive: true });
    await mkdir(path.join(payload, 'resources', 'models'), { recursive: true });
    await writeFile(path.join(payload, 'ayla'), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
    await writeFile(path.join(payload, 'resources', 'app.asar'), 'first');
    await writeFile(path.join(payload, 'resources', 'bin', 'ayla-sidecar'), 'backend first');
    await writeFile(path.join(payload, 'resources', 'models', 'model-manifest.json'), '{}');
    await symlink('bin/ayla-sidecar', path.join(payload, 'resources', 'relative-link'));
    const prefix = path.join(directory, 'path with spaces', 'ayla');
    const home = path.join(directory, 'test-home');
    const first = await activatePayload(payload, prefix, '0.1.0', home);
    assert.equal(await readlink(path.join(first.destination, 'resources', 'relative-link')), 'bin/ayla-sidecar');
    // A backend-only fix must create a distinct installation, even with unchanged UI.
    await writeFile(path.join(payload, 'resources', 'bin', 'ayla-sidecar'), 'backend second');
    const second = await activatePayload(payload, prefix, '0.1.0', home);
    assert.notEqual(first.identifier, second.identifier);
    assert.equal(await readlink(path.join(prefix, 'previous')), `versions/${first.identifier}`);
    assert.equal(await readlink(path.join(prefix, 'current')), `versions/${second.identifier}`);
    await activatePayload(payload, prefix, '0.1.0', home);
    assert.equal(await readlink(path.join(prefix, 'previous')), `versions/${first.identifier}`, 'reinstall must preserve the older rollback target');
    const launcher = await readFile(second.launcher, 'utf8');
    assert.match(launcher, /exec '.*path with spaces.*' "\$@"/);
    assert.equal(JSON.parse(await readFile(path.join(prefix, 'installation.json'))).identifier, second.identifier);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
