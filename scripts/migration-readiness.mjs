// Read-only aggregate review. Never retrieves document, prompt, or fact text.
import { existsSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const argument = process.argv.slice(2);
if (argument.length && (argument.length !== 2 || argument[0] !== '--config')) throw new Error('Use --config PATH');
const installed = path.join(process.env.XDG_CONFIG_HOME || path.join(homedir(), '.config'), 'ayla', 'settings.env');
const config = argument[1] || process.env.AYLA_CONFIG_FILE || (existsSync(installed) ? installed : path.join(root, '.env'));
const settings = {};
if (existsSync(config)) {
  for (const line of (await readFile(config, 'utf8')).split('\n')) {
    const match = line.trim().replace(/^export /, '').match(/^([A-Z][A-Z0-9_]*)\s*=\s*(.*)$/);
    if (!match) continue;
    const raw = match[2].trim();
    settings[match[1]] = /^(".*"|'.*')$/.test(raw) ? raw.slice(1, -1) : raw;
  }
} else if (argument[1] || process.env.AYLA_CONFIG_FILE) throw new Error('Explicit configuration file does not exist');
const env = { ...settings, ...Object.fromEntries(Object.entries(process.env).filter(([, value]) => value !== '')) };
const namespace = env.SURREALDB_NS || 'ayla';
const database = env.SURREALDB_DB || 'memory';
if (![namespace, database].every(s => /^[A-Za-z0-9_]{1,128}$/.test(s))) throw new Error('Invalid namespace/database');
const authorization = `Basic ${Buffer.from(`${env.SURREALDB_USER || env.SURREAL_USER || 'root'}:${env.SURREALDB_PASS || env.SURREAL_PASS || ''}`).toString('base64')}`;
const queries = {
  graphNodes: 'SELECT count() AS total FROM graph_node_v2 GROUP ALL;',
  graphEdges: 'SELECT count() AS total FROM graph_edge_v2 GROUP ALL;',
  documents: 'SELECT count() AS total FROM kb_file GROUP ALL;',
  failedApprovals: "SELECT count() AS total FROM knowledge_operation WHERE status = 'failed' GROUP ALL;",
  interruptedApprovals: "SELECT count() AS total FROM knowledge_operation WHERE status = 'approving' GROUP ALL;",
  deletingSources: "SELECT count() AS total FROM kb_file WHERE status = 'deleting' GROUP ALL;",
  legacyFactsWithoutDocumentSource: "SELECT count() AS total FROM fact WHERE source = NONE OR source = '' OR source = 'conversation' GROUP ALL;",
  legacyEpisodesWithoutSource: "SELECT count() AS total FROM episode WHERE source = NONE OR source = '' GROUP ALL;",
  retainedGraphSnapshots: 'SELECT count() AS total FROM graph_import_run_v2 WHERE backup_available = true GROUP ALL;',
  embeddingProfile: 'SELECT model, digest, dimensions FROM embedding_profile:active;',
};
const response = await fetch(`${(env.SURREALDB_URL || 'http://127.0.0.1:8000').replace(/\/$/, '')}/sql`, { method: 'POST', headers: { Authorization: authorization, Accept: 'application/json', 'surreal-ns': namespace, 'surreal-db': database }, body: Object.values(queries).join('\n'), signal: AbortSignal.timeout(30000) });
if (!response.ok) throw new Error(`Read-only storage review failed: HTTP ${response.status}`);
const results = await response.json();
if (!Array.isArray(results) || results.length !== Object.keys(queries).length || results.some(r => r.status !== 'OK')) throw new Error('Read-only storage review returned an invalid query result');
const counts = Object.fromEntries(Object.keys(queries).filter(k => k !== 'embeddingProfile').map((key, i) => [key, results[i].result?.[0]?.total || 0]));
const profile = results.at(-1).result?.[0] || null;
console.log(JSON.stringify({ timestamp: new Date().toISOString(), readOnly: true, counts, embeddingProfile: profile, reviewRequired: {
  backup: 'Close Ayla and export/verify a coordinated snapshot before schema changes. This report is not a backup.',
  unscopedMemory: counts.legacyFactsWithoutDocumentSource + counts.legacyEpisodesWithoutSource > 0,
  pendingOperations: counts.failedApprovals + counts.interruptedApprovals + counts.deletingSources > 0,
  snapshots: counts.retainedGraphSnapshots > 0,
}, limits: ['Counts are a live observation and may change while the application is running.', 'Legacy memory is reported, never erased or assigned speculative provenance.', 'Filename collisions and graph snapshot scalability still need explicit migration designs.'] }, null, 2));
