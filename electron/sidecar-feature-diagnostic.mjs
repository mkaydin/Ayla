import { diagnosticEnvironment, backendCommandCatalog } from './diagnostic-environment.mjs';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import process from 'node:process';
import {
  developmentSidecarPath,
  SidecarManager,
  SidecarRequestError,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const diagnosticId = randomUUID();
const source = `phase4-live-${diagnosticId}.md`;
const entity = `PhaseFourEntity${diagnosticId.replaceAll('-', '').slice(0, 12)}`;
const target = `PhaseFourTarget${diagnosticId.replaceAll('-', '').slice(12, 24)}`;
const text = [
  `# Phase 4 Live Diagnostic`,
  `${entity} is connected to ${target} through the relation validates_desktop_parity.`,
  `${target} is a unique temporary concept used only by the Ayla Phase 4 diagnostic.`,
].join('\n\n');
const manager = new SidecarManager({
  binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root),
  cwd: root,
  env: {
    ...diagnosticEnvironment(),
    AYLA_DEVELOPMENT_ROOT: root,
  },
  startupTimeout: 60_000,
  restartLimit: 0,
});

let fileCreated = false;
const result = {
  status: 'running',
  dbReady: false,
  tts: null,
  chat: null,
  chatCancellation: null,
  searchCancellation: null,
  ingestion: null,
  cleanup: null,
};

function assertWav(value) {
  const bytes = value instanceof Uint8Array ? value : new Uint8Array(value || []);
  const ascii = (start, end) => String.fromCharCode(...bytes.subarray(start, end));
  if (bytes.length <= 44 || ascii(0, 4) !== 'RIFF' || ascii(8, 12) !== 'WAVE') {
    throw new Error(`TTS returned an invalid WAV payload (${bytes.length} bytes)`);
  }
  return bytes.length;
}

try {
  const hello = await manager.start();
  result.protocolVersion = hello.protocolVersion;
  result.backendVersion = hello.backendVersion;
  result.dbReady = await manager.request('check_db_ready');
  if (!result.dbReady) {
    throw new Error('Database initialization is offline; live ingestion parity cannot run');
  }

  const wav = await manager.request('generate_tts', {
    text: 'Phase four live parity check.',
  });
  result.tts = { wavBytes: assertWav(wav), header: 'RIFF/WAVE' };

  const deltas = [];
  await manager.request('stream_chat', {
    messages: [
      {
        role: 'system',
        content: 'Reply with one short sentence confirming a live desktop backend test.',
      },
      { role: 'user', content: 'Confirm Phase 4 live chat parity.' },
    ],
  }, {
    onStream: event => {
      if (event?.type === 'delta' && event.content) deltas.push(event.content);
    },
  });
  const chatText = deltas.join('').trim();
  if (!chatText) throw new Error('Live chat completed without a streamed text delta');
  result.chat = { deltas: deltas.length, chars: chatText.length };

  const cancellationId = randomUUID();
  let cancellationRequested = false;
  const cancelledChat = manager.request('stream_chat', {
    messages: [
      {
        role: 'system',
        content: 'Write a detailed multi-paragraph explanation and do not stop early.',
      },
      { role: 'user', content: 'Explain distributed consensus in extensive detail.' },
    ],
  }, {
    requestId: cancellationId,
    onStream: event => {
      if (!cancellationRequested && event?.type === 'delta') {
        cancellationRequested = true;
        manager.cancelRequest(cancellationId, 'Phase 4 cancellation diagnostic');
      }
    },
  });
  await cancelledChat.then(
    () => {
      throw new Error('Live chat cancellation unexpectedly completed successfully');
    },
    error => {
      if (!(error instanceof SidecarRequestError) || error.code !== 'request_cancelled') {
        throw error;
      }
    },
  );
  result.chatCancellation = {
    requestId: cancellationId,
    result: 'request_cancelled',
    pendingRequests: manager.pending.size,
  };

  const searchHistoryBefore = await manager.request('get_search_history');
  const searchId = randomUUID();
  let searchStarted = null;
  const onSearchEvent = message => {
    if (message.name !== 'search-started' || message.operation_id !== searchId) return;
    searchStarted = message.payload?.pid || null;
    manager.cancelRequest(searchId, 'Phase 4 search cancellation diagnostic');
  };
  manager.on('backend-event', onSearchEvent);
  const cancelledSearch = manager.request('run_search', {
    query: `phase four cancellation diagnostic ${diagnosticId}`,
    mode: 'knowledge',
  }, { requestId: searchId });
  await cancelledSearch.then(
    () => {
      throw new Error('Live search cancellation unexpectedly completed successfully');
    },
    error => {
      if (!(error instanceof SidecarRequestError) || error.code !== 'request_cancelled') {
        throw error;
      }
    },
  );
  manager.off('backend-event', onSearchEvent);
  await new Promise(resolve => setTimeout(resolve, 100));
  const searchHistoryAfter = await manager.request('get_search_history');
  const beforeIds = new Set(searchHistoryBefore.map(item => item.search_id || item.id));
  const newSearchRecords = searchHistoryAfter.filter(
    item => !beforeIds.has(item.search_id || item.id),
  );
  if (!searchStarted || newSearchRecords.length) {
    throw new Error(
      `Search cancellation cleanup failed: pid=${searchStarted}, newRecords=${newSearchRecords.length}`,
    );
  }
  result.searchCancellation = {
    requestId: searchId,
    processId: searchStarted,
    result: 'request_cancelled',
    historyRecordsCreated: 0,
  };

  await manager.request('save_file_record', {
    name: source,
    path: `diagnostic://${diagnosticId}`,
    chars: text.length,
    sourceType: 'phase4_diagnostic',
    text,
  });
  fileCreated = true;
  const ingestResult = await manager.request('ingest_text', {
    text,
    source,
    sourceType: 'phase4_diagnostic',
  });
  const [filesAfterIngest, graphAfterIngest] = await Promise.all([
    manager.request('get_files'),
    manager.request('get_kb_data'),
  ]);
  if (!filesAfterIngest.some(file => file.name === source)) {
    throw new Error('Live ingestion did not persist its diagnostic file record');
  }
  const sourceNodes = graphAfterIngest.nodes.filter(node => (
    node.source === source || node.sources?.includes(source)
  ));
  if (!sourceNodes.length) {
    throw new Error('Live ingestion did not produce a source-linked knowledge node');
  }
  result.ingestion = {
    result: String(ingestResult),
    sourceNodes: sourceNodes.length,
    graphNodes: graphAfterIngest.nodes.length,
    graphEdges: graphAfterIngest.edges.length,
  };

  await manager.request('delete_file_record', { name: source, deleteNodes: true });
  fileCreated = false;
  const [filesAfterDelete, graphAfterDelete] = await Promise.all([
    manager.request('get_files'),
    manager.request('get_kb_data'),
  ]);
  const sourceRemains = graphAfterDelete.nodes.some(node => (
    node.source === source || node.sources?.includes(source)
  ));
  if (filesAfterDelete.some(file => file.name === source) || sourceRemains) {
    throw new Error('Diagnostic ingestion data remains after cleanup');
  }
  result.cleanup = {
    fileRemaining: false,
    sourceLinkedNodeRemaining: false,
  };

  await manager.stop();
  result.status = 'ok';
  result.cleanShutdown = manager.snapshot().state === 'stopped';
  console.log(JSON.stringify(result, null, 2));
} catch (error) {
  if (fileCreated) {
    await manager.request('delete_file_record', { name: source, deleteNodes: true })
      .catch(cleanupError => {
        error.message += `; diagnostic cleanup also failed: ${cleanupError.message}`;
      });
  }
  await manager.stop().catch(() => {});
  throw error;
}
