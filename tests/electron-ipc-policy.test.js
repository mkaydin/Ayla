import assert from 'node:assert/strict';
import test from 'node:test';
import {
  isTrustedSenderUrl,
  normalizeDialogOptions,
  normalizeNotification,
  validateBackendCall,
  validateRequestId,
} from '../electron/ipc-policy.mjs';

test('Electron IPC origin policy rejects malformed and lookalike origins', () => {
  const origin = 'http://127.0.0.1:4173';
  assert.equal(isTrustedSenderUrl(`${origin}/index.html`, origin), true);
  assert.equal(isTrustedSenderUrl('http://127.0.0.1:4174/', origin), false);
  assert.equal(isTrustedSenderUrl('http://127.0.0.1:4173.evil.test/', origin), false);
  assert.equal(isTrustedSenderUrl('not a URL', origin), false);
});

test('Electron backend policy validates command, argument, and payload boundaries', () => {
  assert.doesNotThrow(() => validateBackendCall('get_kb_data', {}, 32));
  assert.throws(() => validateBackendCall('../shell', {}, 32), /Invalid Rust backend command/);
  assert.throws(() => validateBackendCall('get_kb_data', [], 32), /arguments must be an object/);
  assert.throws(
    () => validateBackendCall('ingest_text', { text: 'x'.repeat(64) }, 32),
    /payload limit/,
  );
});

test('Electron file dialog policy sanitizes extensions and bounds filters', () => {
  const options = normalizeDialogOptions({
    multiple: false,
    filters: [
      { name: 'Documents', extensions: ['.pdf', '../md', 'tar.gz', ''] },
      { name: 'Empty', extensions: [] },
    ],
  });
  assert.deepEqual(options.properties, ['openFile']);
  assert.deepEqual(options.filters, [{
    name: 'Documents',
    extensions: ['pdf', 'md', 'targz'],
  }]);
});

test('Electron request identifiers accept UUIDs and reject channel injection', () => {
  assert.equal(
    validateRequestId('11111111-1111-4111-8111-111111111111'),
    '11111111-1111-4111-8111-111111111111',
  );
  assert.throws(() => validateRequestId('short'), /Invalid Rust backend request/);
  assert.throws(
    () => validateRequestId('11111111-1111-4111-8111-111111111111:other'),
    /Invalid Rust backend request/,
  );
});

test('Electron notification policy bounds text and restricts routing fields', () => {
  const payload = normalizeNotification({
    title: 'Research\ncomplete',
    body: 'A'.repeat(400),
    target: '../shell',
    urgency: 'unknown',
    operationId: 'search/id?<bad>',
    dedupeKey: 'search:complete:id<script>',
  });
  assert.equal(payload.title, 'Research complete');
  assert.equal(payload.body.length, 300);
  assert.equal(payload.target, 'status');
  assert.equal(payload.urgency, 'normal');
  assert.equal(payload.operationId, 'searchidbad');
  assert.equal(payload.dedupeKey, 'search:complete:idscript');
  assert.throws(() => normalizeNotification({ title: '', body: '' }), /required/);
});
