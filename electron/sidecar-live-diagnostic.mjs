import path from 'node:path';
import process from 'node:process';
import {
  developmentSidecarPath,
  SidecarManager,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const manager = new SidecarManager({
  binaryPath: developmentSidecarPath(root),
  cwd: root,
  env: {
    ...process.env,
    ALICE_DEVELOPMENT_ROOT: path.join(root, 'src-tauri'),
  },
  startupTimeout: 60_000,
  restartLimit: 0,
});

try {
  const hello = await manager.start();
  await manager.ping();
  const ready = await manager.request('check_db_ready');
  if (!ready) {
    throw new Error(
      'Rust sidecar started, but SurrealDB/Qdrant initialization is offline. Check Docker services.',
    );
  }

  const [graph, files, sessions, searches, extracted] = await Promise.all([
    manager.request('get_kb_data'),
    manager.request('get_files'),
    manager.request('load_chat_sessions'),
    manager.request('get_search_history'),
    manager.request('run_quick_extract', { filePath: path.join(root, 'package.json') }),
  ]);
  await manager.stop();

  console.log(JSON.stringify({
    status: 'ok',
    protocolVersion: hello.protocolVersion,
    backendVersion: hello.backendVersion,
    dbReady: ready,
    graphNodes: graph.nodes.length,
    graphEdges: graph.edges.length,
    files: files.length,
    chatSessions: sessions.length,
    searchHistory: searches.length,
    quickExtractChars: extracted.chars,
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  await manager.stop().catch(() => {});
  throw error;
}
