import { randomUUID } from 'node:crypto';
import path from 'node:path';
import { diagnosticEnvironment } from './diagnostic-environment.mjs';
import { SidecarManager, developmentSidecarPath } from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const env = { ...diagnosticEnvironment(), AYLA_DEVELOPMENT_ROOT: root };
const manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root, env, startupTimeout: 60000 });
const name = `release-document-${randomUUID()}.md`;
const text = '# AylaPressureSensor\n\nAylaPressureSensor measures pressure. Its calibrated pressure is 42.75 MPa.\n';
try {
  await manager.start();
  if (!await manager.request('check_db_ready')) throw new Error('Isolated test storage is unavailable');
  await manager.request('commit_document', { name, path: `fixture://${name}`, chars: text.length, sourceType: 'diagnostic', text });
  await manager.restart();
  if (await manager.request('get_document_text', { name }) !== text) throw new Error('Committed text did not survive restart');
  const context = await manager.request('enrich_prompt', { userQuery: 'AylaPressureSensor calibrated pressure', basePrompt: 'You are Ayla.' });
  if (!String(context).includes('42.75') || !String(context).includes(name)) throw new Error('Committed document is not retrieved with provenance');
  await manager.request('delete_file_record', { name, deleteNodes: true });
  const deletedGraph = await manager.request('get_graph_v2_data');
  const deletionBackup = deletedGraph.runs.find(run => run.action === 'legacy_sync')?.backup_id;
  if (!deletionBackup) throw new Error('Deletion has no recoverable graph backup');
  await manager.request('rollback_graph_v2', { backupId: deletionBackup });
  const rolledContext = await manager.request('enrich_prompt', { userQuery: 'AylaPressureSensor', basePrompt: 'You are Ayla.' });
  if (String(rolledContext).includes('42.75')) throw new Error('Graph-only rollback revived a deleted document in retrieval');
  await manager.request('delete_file_nodes', { name });
  await manager.restart();
  if (await manager.request('get_document_text', { name })) throw new Error('Deleted text reappeared after restart');
  if ((await manager.request('get_files')).some(file => file.name === name)) throw new Error('Deleted file metadata survived');
  const graph = await manager.request('get_graph_v2_data');
  if (graph.nodes.some(node => node.sources.includes(name))) throw new Error('Deleted graph provenance survived');
  const remaining = await manager.request('search_documents', { query: 'AylaPressureSensor', limit: 10 });
  if (remaining.some(result => JSON.stringify(result).includes('42.75'))) throw new Error('Deleted vector content survived');
  manager.env.AYLA_EMBED_MODEL = 'ayla-diagnostic-different-embedding:never';
  await manager.restart();
  if (await manager.request('check_db_ready')) throw new Error('Changed embedding identity reused existing storage');
  manager.env.AYLA_EMBED_MODEL = 'qwen3-embedding:0.6b';
  // Fail the embedding transport while retaining the recorded model identity.
  const originalOllama = manager.env.OLLAMA_URL;
  manager.env.OLLAMA_URL = 'http://127.0.0.1:1';
  await manager.restart();
  try {
    await manager.request('commit_document', { name, path: `fixture://${name}`, chars: text.length, sourceType: 'diagnostic', text });
    throw new Error('Unavailable embedding endpoint unexpectedly committed');
  } catch (error) { if (error.message === 'Unavailable embedding endpoint unexpectedly committed') throw error; }
  if (!(await manager.request('get_files')).some(file => file.name === name && file.status === 'failed')) throw new Error('Failed approval input is not recoverable');
  if (await manager.request('get_document_text', { name }) !== text) throw new Error('Failed input cannot be previewed for retry');
  if (originalOllama == null) delete manager.env.OLLAMA_URL;
  else manager.env.OLLAMA_URL = originalOllama;
  await manager.restart();
  if (String(await manager.request('enrich_prompt', { userQuery: 'AylaPressureSensor', basePrompt: 'You are Ayla.' })).includes('42.75')) throw new Error('Failed approval leaked into retrieval');
  await manager.request('commit_document', { name, path: `fixture://${name}`, chars: text.length, sourceType: 'diagnostic', text });
  await manager.request('delete_file_record', { name, deleteNodes: true });
  await manager.stop();
  console.log(JSON.stringify({ status: 'ok', commitRestartRetrieval: true, durableDeletion: true, failedCommitRecovery: true, embeddingIdentityGuard: true, rollbackExclusion: true, cleanShutdown: true }, null, 2));
} finally { await manager.stop(); }
