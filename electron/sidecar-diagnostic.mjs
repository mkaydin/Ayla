import { diagnosticEnvironment, backendCommandCatalog } from './diagnostic-environment.mjs';
import path from 'node:path';
import process from 'node:process';
import {
  developmentSidecarPath,
  SidecarManager,
  SidecarRequestError,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const manager = new SidecarManager({
  binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root),
  args: ['--offline'],
  cwd: root,
  env: {
    ...diagnosticEnvironment(),
    AYLA_DEVELOPMENT_ROOT: root,
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
  const expected = backendCommandCatalog(root);
  const missing = expected.filter(method => !manager.methods.has(method));
  const unexpected = [...manager.methods].filter(method => !expected.includes(method));
  if (missing.length || unexpected.length) {
    throw new Error(`Backend catalog mismatch: missing=${missing}, unexpected=${unexpected}`);
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
  const initialGraphV2 = await manager.request('get_graph_v2_data');
  if (initialGraphV2?.available !== false
      || !Array.isArray(initialGraphV2?.nodes)
      || !Array.isArray(initialGraphV2?.runs)) {
    throw new Error(
      `get_graph_v2_data did not preserve its offline contract: ${JSON.stringify(initialGraphV2)}`,
    );
  }

  const extracted = await manager.request('run_quick_extract', {
    filePath: path.join(root, 'package.json'),
  });
  if (extracted?.filename !== 'package.json' || extracted.chars <= 0) {
    throw new Error(`Quick Extract returned an invalid payload: ${JSON.stringify(extracted)}`);
  }

  await expectError('ingest_text', {
    text: 'Ayla sidecar diagnostic document.', source: 'sidecar-diagnostic.md', sourceType: 'diagnostic',
  }, 'db_offline');
  const graph = await manager.request('get_kb_data');
  if (graph.nodes.length !== initialGraph.nodes.length) throw new Error('Offline approval changed knowledge');

  const cancelled = await manager.request('cancel_process', { pid: 'missing-process' });
  if (cancelled !== false) {
    throw new Error(`Missing process cancellation returned ${cancelled}`);
  }

  await manager.request('stop_tts');
  let ttsContract = 'available';
  try {
    const wav = await manager.request('generate_tts', { text: 'hello' });
    if (!(wav instanceof Uint8Array) || wav.length < 44 || String.fromCharCode(...wav.slice(0, 4)) !== 'RIFF') {
      throw new Error('generate_tts did not return a valid binary WAV');
    }
  } catch (error) {
    if (!(error instanceof SidecarRequestError) || error.code !== 'tts_generation') throw error;
    ttsContract = 'unavailable-explicitly';
  }
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
    offlineIngest: 'rejected-explicitly',
    graphNodes: graph.nodes.length,
    graphEdges: graph.edges.length,
    offlineGraphV2: 'unavailable-explicitly',
    cancellation: 'ok',
    ttsContract,
    mutationErrorContract: 'ok',
    allowlist,
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  await manager.stop().catch(() => {});
  throw error;
}
