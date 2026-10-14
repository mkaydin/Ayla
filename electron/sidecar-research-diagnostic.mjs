import path from 'node:path';
import { diagnosticEnvironment } from './diagnostic-environment.mjs';
import { SidecarManager, developmentSidecarPath } from './sidecar-manager.mjs';
const root = path.resolve(import.meta.dirname, '..');
// The fixture contains only the explicitly requested public research topic.
const env = { ...diagnosticEnvironment(), AYLA_DEVELOPMENT_ROOT: root, AYLA_OLLAMA_LONG_TIMEOUT_SECS: '180' };
const manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root, env, startupTimeout: 60000 });
const outcomes = [];
manager.on('backend-event', message => {
  if (message.name === 'search-progress') console.error(JSON.stringify({ stage: message.payload?.stage, percent: message.payload?.percent }));
});
try {
  await manager.start();
  for (const mode of ['knowledge', 'papers']) {
    const result = await manager.request('run_search', { query: 'Young modulus elastic stress strain relationship', mode });
    if (!result.persisted || !result.search_id || !result.knowledge_document?.includes('http')) throw new Error(`${mode}: missing durable report/citations`);
    if (!(result.total_results_found > 0)) throw new Error(`${mode}: no discovered sources`);
    if (mode === 'papers' && (!result.documents?.length || result.documents.some(document => !document.source_url?.startsWith('http') || !/^[a-f0-9]{64}$/.test(document.sha256 || '')))) throw new Error('Downloaded PDFs lost their exact URL/hash provenance');
    outcomes.push({ mode, status: result.status || 'complete', sources: result.total_results_found, downloadedPdfs: result.pdfs_downloaded, warnings: result.warnings || [] });
  }
  await manager.restart();
  const history = await manager.request('get_search_history');
  if (history.length !== 2 || history.some(report => !['knowledge', 'papers'].includes(report.mode))) throw new Error('Research history did not survive restart');
  await manager.stop();
  console.log(JSON.stringify({ status: 'ok', liveModes: outcomes, durableHistory: true, sourceLinkedReports: true, downloadedPdfProvenance: true }, null, 2));
} finally { await manager.stop(); }
