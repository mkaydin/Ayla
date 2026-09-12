import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { randomUUID } from 'node:crypto';

// Every diagnostic process owns its structured storage, vectors, and artifacts.
// Service credentials/model choices are inherited, but user data never is.
export function diagnosticEnvironment() {
  const id = randomUUID().replaceAll('-', '');
  return {
    ...process.env,
    AYLA_DATA_DIR: mkdtempSync(path.join(tmpdir(), 'ayla-diagnostic-')),
    SURREALDB_NS: 'ayla_diagnostics',
    SURREALDB_DB: `run_${id}`,
    SURREALDB_URL: process.env.AYLA_DIAGNOSTIC_SURREAL_URL || 'http://127.0.0.1:18000',
    SURREALDB_USER: process.env.AYLA_DIAGNOSTIC_SURREAL_USER || 'ayla_test',
    SURREALDB_PASS: process.env.AYLA_DIAGNOSTIC_SURREAL_PASS || 'isolated_test_only',
    QDRANT_URL: process.env.AYLA_DIAGNOSTIC_QDRANT_URL || 'http://127.0.0.1:16334',
    AYLA_QDRANT_COLLECTION_PREFIX: `diagnostic_${id}_`,
  };
}

export function backendCommandCatalog(root) {
  const source = readFileSync(path.join(root, 'crates/ayla-core/src/commands.rs'), 'utf8');
  const catalog = source.match(/BACKEND_METHODS[^=]*=\s*&\[([\s\S]*?)\];/);
  if (!catalog) throw new Error('Cannot read the shared backend command catalog');
  return [...catalog[1].matchAll(/"([a-z][a-z0-9_]*)"/g)].map(match => match[1]);
}
