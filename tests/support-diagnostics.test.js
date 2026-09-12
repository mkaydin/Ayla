import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { collectSupportDiagnostics, writeDiagnostic } from '../electron/support-diagnostics.mjs';

test('support diagnostics rotate and exclude private data and untrusted fields', () => {
  const root = mkdtempSync(join(tmpdir(), 'ayla-support-test-'));
  try {
    for (let index = 0; index < 50; index++) writeDiagnostic(root, { event: 'backend-status', state: 'ready', protocolVersion: 1, error: 'secret password', prompt: 'private document', appVersion: 'token://secret', restartAttempt: index }, 500);
    const files = readdirSync(join(root, 'logs'));
    assert.equal(files.length, 3);
    for (const file of files) {
      assert.ok(statSync(join(root, 'logs', file)).size <= 500);
      const text = readFileSync(join(root, 'logs', file), 'utf8');
      assert.ok(!text.includes('secret') && !text.includes('private'));
      assert.equal(statSync(join(root, 'logs', file)).mode & 0o777, 0o600);
    }
    const bundle = collectSupportDiagnostics(root);
    assert.ok(bundle.runtimeEvents.length > 0 && bundle.runtimeEvents.length < 50);
    assert.ok(!JSON.stringify(bundle).includes(root));
  } finally { rmSync(root, { recursive: true, force: true }); }
});
