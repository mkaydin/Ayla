import assert from 'node:assert/strict';
import test from 'node:test';

async function loadAdapter(name, windowValue) {
  globalThis.window = windowValue;
  return import(`../src/lib/ipc.js?adapter-test=${name}-${Date.now()}-${Math.random()}`);
}

test('desktop adapter routes Electron commands without exposing preload details to UI', async () => {
  const calls = [];
  const adapter = await loadAdapter('electron', {
    aliceDesktop: {
      runtime: 'electron',
      invoke: async (command, args) => {
        calls.push({ command, args });
        return { ok: true };
      },
      runtimeInfo: async () => ({
        runtime: 'electron',
        electron: 'fixture',
        chrome: 'fixture',
      }),
      stream: async (command, args, onEvent, requestId) => {
        calls.push({ command, args, requestId });
        onEvent({ type: 'delta', content: 'fixture' });
      },
      cancelStream: async requestId => {
        calls.push({ cancelled: requestId });
        return true;
      },
    },
  });

  assert.equal(adapter.getDesktopRuntime(), 'electron');
  assert.equal(adapter.hasDesktopBackend(), true);
  assert.equal(adapter.serviceHealthUrl('ollama', 'http://localhost'), '/api/services/ollama');
  assert.deepEqual(await adapter.invoke('check_db_ready', { value: 1 }), { ok: true });
  const deltas = [];
  const stream = adapter.invokeStreamCancelable(
    'stream_chat',
    { messages: [] },
    event => deltas.push(event.content),
  );
  await stream.promise;
  assert.deepEqual(deltas, ['fixture']);
  assert.equal(calls[0].command, 'check_db_ready');
  assert.equal(calls[1].requestId, stream.requestId);
});

test('desktop adapter retains Tauri command compatibility behind the same API', async () => {
  const calls = [];
  const adapter = await loadAdapter('tauri', {
    __TAURI_INTERNALS__: {
      invoke: async (command, args) => {
        calls.push({ command, args });
        return 'tauri-result';
      },
    },
  });

  assert.equal(adapter.getDesktopRuntime(), 'tauri');
  assert.equal(adapter.hasDesktopBackend(), true);
  assert.equal(
    adapter.serviceHealthUrl('ollama', 'http://localhost:11434/api/tags'),
    'http://localhost:11434/api/tags',
  );
  assert.equal(await adapter.invoke('check_db_ready'), 'tauri-result');
  assert.equal(calls[0].command, 'check_db_ready');
});

test('desktop adapter reports unavailable browser mode instead of simulating backend work', async () => {
  const adapter = await loadAdapter('browser', {});
  assert.equal(adapter.getDesktopRuntime(), 'browser');
  assert.equal(adapter.hasDesktopBackend(), false);
  await assert.rejects(
    adapter.invoke('get_kb_data'),
    /Rust command backend is not available in browser mode/,
  );
});
