import path from 'node:path';
import process from 'node:process';
import {
  developmentSidecarPath,
  SidecarManager,
  SidecarRequestError,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const manager = new SidecarManager({
  binaryPath: developmentSidecarPath(root),
  args: ['--offline'],
  cwd: root,
  env: {
    ...process.env,
    AYLA_DEVELOPMENT_ROOT: path.join(root, 'src-tauri'),
  },
  startupTimeout: 5_000,
});

async function expectError(method, params, expectedCode) {
  try {
    await manager.request(method, params);
  } catch (error) {
    if (error instanceof SidecarRequestError && error.code === expectedCode) return error;
    throw new Error(
      `${method} returned ${error.code ?? error.name}; expected ${expectedCode}: ${error.message}`,
    );
  }
  throw new Error(`${method} unexpectedly succeeded; expected ${expectedCode}`);
}

try {
  const hello = await manager.start();
  if (manager.methods.size !== 29) {
    throw new Error(`Expected 29 advertised methods, got ${manager.methods.size}`);
  }
  await manager.ping();

  const ready = await manager.request('check_db_ready');
  if (ready !== false) {
    throw new Error(`Offline backend unexpectedly reported database ready: ${ready}`);
  }

  const initialGraph = await manager.request('get_kb_data');
  if (!Array.isArray(initialGraph?.nodes) || !Array.isArray(initialGraph?.edges)) {
    throw new Error(
      `get_kb_data did not return a graph payload: ${JSON.stringify(initialGraph)}`,
    );
  }

  const extracted = await manager.request('run_quick_extract', {
    filePath: path.join(root, 'package.json'),
  });
  if (extracted?.filename !== 'package.json' || extracted.chars <= 0) {
    throw new Error(`Quick Extract returned an invalid payload: ${JSON.stringify(extracted)}`);
  }

  const ingestResult = await manager.request('ingest_text', {
    text: 'Ayla sidecar diagnostic document.',
    source: 'sidecar-diagnostic.md',
    sourceType: 'diagnostic',
  });
  if (typeof ingestResult !== 'string') {
    throw new Error(`ingest_text returned an invalid payload: ${JSON.stringify(ingestResult)}`);
  }

  const graph = await manager.request('get_kb_data');
  if (!graph.nodes.some(node => node.name === 'sidecar-diagnostic.md')) {
    throw new Error('Offline ingest did not update the shared in-process graph');
  }

  const cancelled = await manager.request('cancel_process', { pid: 'missing-process' });
  if (cancelled !== false) {
    throw new Error(`Missing process cancellation returned ${cancelled}`);
  }

  await manager.request('stop_tts');
  await expectError('generate_tts', { text: 'hello' }, 'tts_generation');
  await expectError(
    'save_chat_session',
    { sessionId: 'diagnostic', title: 'Diagnostic', messagesJson: '[]' },
    'db_offline',
  );

  let allowlist = 'ok';
  try {
    await manager.request('not_a_command');
    throw new Error('Unknown command unexpectedly succeeded');
  } catch (error) {
    if (!error.message.includes("does not advertise 'not_a_command'")) throw error;
  }

  await manager.stop();
  console.log(JSON.stringify({
    status: 'ok',
    protocolVersion: hello.protocolVersion,
    backendVersion: hello.backendVersion,
    advertisedMethods: manager.methods.size,
    ping: 'ok',
    offlineDbReady: ready,
    quickExtractChars: extracted.chars,
    offlineIngest: 'ok',
    graphNodes: graph.nodes.length,
    graphEdges: graph.edges.length,
    cancellation: 'ok',
    ttsErrorContract: 'ok',
    mutationErrorContract: 'ok',
    allowlist,
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  await manager.stop().catch(() => {});
  throw error;
}
