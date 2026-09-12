import assert from 'node:assert/strict';
import test from 'node:test';
import { startStorageRecovery } from '../electron/storage-recovery.mjs';

test('storage recovery waits for idle and never interrupts active work', async () => {
  let scheduled;
  let restarts = 0;
  let checks = 0;
  const backend = {
    pending: new Map([['active-chat', {}]]),
    snapshot: () => ({ backendAvailable: true }),
    request: async () => { checks++; return false; },
    restart: async () => { restarts++; },
  };
  const stop = startStorageRecovery(backend, {
    setTimer: callback => { scheduled = callback; return 1; },
    clearTimer: () => {},
  });
  await scheduled();
  assert.equal(checks, 0);
  assert.equal(restarts, 0);
  backend.pending.clear();
  await scheduled();
  assert.equal(restarts, 1);
  stop();
  await scheduled();
  assert.equal(restarts, 1);
});

test('storage recovery preserves an active dictation session', async () => {
  let scheduled;
  let restarts = 0;
  const backend = {
    pending: new Map(), snapshot: () => ({ backendAvailable: true }),
    request: async method => method === 'check_db_ready' ? false : { stt: { listening: true } },
    restart: async () => { restarts++; },
  };
  const stop = startStorageRecovery(backend, { setTimer: callback => { scheduled = callback; return 1; }, clearTimer: () => {} });
  await scheduled(); assert.equal(restarts, 0); stop();
});
