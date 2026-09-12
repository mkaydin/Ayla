import { randomUUID } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { diagnosticEnvironment } from './diagnostic-environment.mjs';
import { SidecarManager, developmentSidecarPath } from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const directory = mkdtempSync(path.join(tmpdir(), 'ayla-backup-diagnostic-'));
const env = { ...diagnosticEnvironment(), AYLA_DEVELOPMENT_ROOT: root, QDRANT_HTTP_URL: 'http://127.0.0.1:16333' };
const config = path.join(directory, 'settings.env');
const configuredKeys = ['SURREALDB_URL', 'SURREALDB_USER', 'SURREALDB_PASS', 'SURREALDB_NS', 'SURREALDB_DB', 'QDRANT_URL', 'QDRANT_HTTP_URL', 'AYLA_QDRANT_COLLECTION_PREFIX'];
writeFileSync(config, configuredKeys.map(key => `export ${key}="${env[key]}"`).join('\n'), { mode: 0o600 });
env.AYLA_CONFIG_FILE = config;
// Fixture extraction stays on the user's already-authorized cloud model; no private source is used.
env.AYLA_LLM_MODEL = 'nemotron-3-super:cloud';
let manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root, env, startupTimeout: 60000 });
function backup(args) {
  const backupEnvironment = { ...env };
  for (const key of configuredKeys) delete backupEnvironment[key];
  const result = spawnSync(process.execPath, ['scripts/knowledge-backup.mjs', ...args], { cwd: root, env: backupEnvironment, encoding: 'utf8', timeout: 300000 });
  if (result.status !== 0) throw new Error(`Snapshot command failed: ${result.stderr || result.error}`);
  return JSON.parse(result.stdout.trim());
}
try {
  await manager.start();
  await manager.request('commit_document', { name: 'backup-fixture.md', path: 'fixture://backup', chars: 86, sourceType: 'diagnostic', text: `BackupSensor measures calibrated pressure. BackupSensor has a pressure of 42.75 MPa. Literal document evidence mentions ${env.AYLA_DATA_DIR}.` });
  await manager.stop();
  const exported = backup(['export', '--output', directory]);
  backup(['verify', '--snapshot', exported.snapshot]);
  const database = `restored_${randomUUID().replaceAll('-', '')}`;
  const prefix = `restored_${randomUUID().replaceAll('-', '')}_`;
  const restoredRoot = path.join(directory, 'restored-data');
  backup(['restore', '--snapshot', exported.snapshot, '--database', database, '--collection-prefix', prefix, '--data-dir', restoredRoot]);
  manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root, env: { ...env, SURREALDB_DB: database, AYLA_QDRANT_COLLECTION_PREFIX: prefix, AYLA_DATA_DIR: restoredRoot }, startupTimeout: 60000 });
  await manager.start();
  const files = await manager.request('get_files');
  if (!(await manager.request('get_document_text', { name: 'backup-fixture.md' })).includes('42.75') || !files.some(file => file.name === 'backup-fixture.md')) throw new Error('Restored structured document is missing');
  if (!(await manager.request('get_document_text', { name: 'backup-fixture.md' })).includes(env.AYLA_DATA_DIR)) throw new Error('Restore rewrote literal document evidence');
  const context = await manager.request('enrich_prompt', { userQuery: 'BackupSensor pressure', basePrompt: 'You are Ayla.' });
  if (!String(context).includes('42.75')) throw new Error('Restored vectors/graph are not retrievable');
  await manager.stop();
  manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root, env, startupTimeout: 60000 });
  await manager.start();
  if (!(await manager.request('get_document_text', { name: 'backup-fixture.md' })).includes('42.75') || !(await manager.request('get_files')).some(file => file.name === 'backup-fixture.md')) throw new Error('Original snapshot source was changed');
  await manager.stop();
  console.log(JSON.stringify({ status: 'ok', coordinatedExport: true, verifiedFiles: exported.verifiedFiles, freshRestore: true, restoredRetrieval: true, originalStorePreserved: true }, null, 2));
} finally { await manager.stop(); }
