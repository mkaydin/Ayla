// Run with the application closed. An offline sidecar holds the same writer
// lock throughout the export/restore; it performs no startup schema migration.
import { createHash, randomUUID } from 'node:crypto';
import { createReadStream, createWriteStream, existsSync } from 'node:fs';
import { cp, mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';
import { pipeline } from 'node:stream/promises';
import { Transform } from 'node:stream';
import { SidecarManager, developmentSidecarPath } from '../electron/sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const [action, ...argv] = process.argv.slice(2);
const options = {};
for (let i = 0; i < argv.length; i += 2) {
  if (!argv[i].startsWith('--') || !argv[i + 1]) throw new Error('Use --option value pairs');
  options[argv[i].slice(2)] = argv[i + 1];
}
if (!['export', 'verify', 'restore'].includes(action)) throw new Error('Use export --output DIR, verify --snapshot DIR, or restore --snapshot DIR --database NEW_DB --collection-prefix NEW_PREFIX --data-dir NEW_DIR');
const defaultDataRoot = path.join(process.env.XDG_CONFIG_HOME || path.join(homedir(), '.config'), 'ayla');
const configPath = options.config || process.env.AYLA_CONFIG_FILE || path.join(defaultDataRoot, 'settings.env');
const settings = {};
if (existsSync(configPath)) {
  for (let line of (await readFile(configPath, 'utf8')).split('\n')) {
    line = line.trim().replace(/^export /, '');
    if (!line.trim() || line.trim().startsWith('#')) continue;
    const at = line.indexOf('=');
    if (at > 0) {
      let value = line.slice(at + 1).trim();
      if (value.length >= 2 && ((value.startsWith('"') && value.endsWith('"')) || (value.startsWith("'") && value.endsWith("'")))) value = value.slice(1, -1);
      settings[line.slice(0, at).trim()] = value;
    }
  }
}
const env = { ...settings, ...Object.fromEntries(Object.entries(process.env).filter(([, value]) => value !== '')) };
// Match Electron's stable data root. Selecting another settings file must not
// move the writer lock or make an export capture the wrong artifacts.
const dataRoot = path.resolve(process.env.AYLA_DATA_DIR || defaultDataRoot);
const surreal = (env.SURREALDB_URL || 'http://127.0.0.1:8000').replace(/\/$/, '');
const qdrant = (env.QDRANT_HTTP_URL || 'http://127.0.0.1:6333').replace(/\/$/, '');
const ns = env.SURREALDB_NS || 'ayla';
const db = env.SURREALDB_DB || 'memory';
const prefix = env.AYLA_QDRANT_COLLECTION_PREFIX || '';
const basic = Buffer.from(`${env.SURREALDB_USER || env.SURREAL_USER || 'root'}:${env.SURREALDB_PASS || env.SURREAL_PASS || ''}`).toString('base64');
const surrealHeaders = { Authorization: `Basic ${basic}`, 'surreal-ns': ns, 'surreal-db': db, Accept: 'application/json' };
const qdrantHeaders = env.QDRANT_API_KEY ? { 'api-key': env.QDRANT_API_KEY } : {};
const maximumFile = 512 * 1024 * 1024;

async function sha256(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}
async function files(directory, relative = '') {
  const result = [];
  for (const entry of await readdir(path.join(directory, relative), { withFileTypes: true })) {
    if (entry.isSymbolicLink()) throw new Error('Snapshots may not contain symlinks');
    const name = path.posix.join(relative, entry.name);
    if (entry.isDirectory()) result.push(...await files(directory, name));
    else if (entry.isFile()) result.push(name);
  }
  return result;
}
async function request(url, init = {}) {
  const response = await fetch(url, { ...init, signal: AbortSignal.timeout(300000) });
  if (!response.ok) throw new Error(`Storage operation failed with HTTP ${response.status}`);
  return response;
}
async function download(url, file, headers, limit = maximumFile) {
  const response = await request(url, { headers });
  let size = 0;
  const bounded = new Transform({ transform(chunk, encoding, callback) {
    size += chunk.length;
    callback(size > limit ? new Error(`Snapshot file exceeds the ${Math.round(limit / 1024 / 1024)} MiB limit`) : null, chunk);
  } });
  await pipeline(response.body, bounded, createWriteStream(file, { flags: 'wx', mode: 0o600 }));
}
async function verify(snapshot) {
  const manifest = JSON.parse(await readFile(path.join(snapshot, 'manifest.json'), 'utf8'));
  if (manifest.format !== 1 || !Array.isArray(manifest.files)) throw new Error('Unsupported snapshot manifest');
  if (!Array.isArray(manifest.collections) || manifest.collections.length > 3 || manifest.collections.some(name => !['documents', 'code_analysis', 'episodes'].includes(name))) throw new Error('Invalid snapshot collections');
  const entries = await files(snapshot);
  if (new Set(manifest.files.map(file => file.path)).size !== manifest.files.length) throw new Error('Duplicate snapshot file entries');
  for (const file of manifest.files) {
    if (file.path.startsWith('/') || file.path.split('/').some(part => !part || part === '..') || !/^[a-f0-9]{64}$/.test(file.sha256)) throw new Error('Unsafe snapshot file record');
    if (!entries.includes(file.path) || await sha256(path.join(snapshot, file.path)) !== file.sha256) throw new Error(`Snapshot checksum mismatch: ${file.path}`);
  }
  const declared = new Set(manifest.files.map(file => file.path));
  if (!declared.has('database.surql') || manifest.collections.some(name => !declared.has(`vectors/${name}.snapshot`)) || entries.some(name => name !== 'manifest.json' && !declared.has(name))) throw new Error('Incomplete or undeclared snapshot files');
  return manifest;
}

if (action === 'verify') {
  const snapshot = path.resolve(options.snapshot || '');
  const manifest = await verify(snapshot);
  console.log(JSON.stringify({ status: 'ok', verifiedFiles: manifest.files.length }));
} else {
  const guardian = new SidecarManager({ binaryPath: env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), args: ['--offline'], cwd: root,
    env: { ...env, AYLA_CONFIG_FILE: configPath, AYLA_DATA_DIR: dataRoot, AYLA_DEVELOPMENT_ROOT: root }, startupTimeout: 30000 });
  try {
    await guardian.start();
    if (action === 'export') {
      if (!options.output) throw new Error('Select an --output directory');
      const parent = path.resolve(options.output);
      await mkdir(parent, { recursive: true, mode: 0o700 });
      const snapshot = path.join(parent, `ayla-${randomUUID()}`);
      const staging = `${snapshot}.partial`;
      await mkdir(path.join(staging, 'vectors'), { recursive: true, mode: 0o700 });
      try {
        await download(`${surreal}/export`, path.join(staging, 'database.surql'), surrealHeaders, 128 * 1024 * 1024);
        const available = (await (await request(`${qdrant}/collections`, { headers: qdrantHeaders })).json()).result.collections.map(item => item.name);
        const collections = [];
        for (const logical of ['documents', 'code_analysis', 'episodes']) {
          const collection = `${prefix}${logical}`;
          if (!available.includes(collection)) continue;
          const created = await (await request(`${qdrant}/collections/${encodeURIComponent(collection)}/snapshots`, { method: 'POST', headers: qdrantHeaders })).json();
          const name = created.result.name;
          if (!/^[a-zA-Z0-9._-]+$/.test(name)) throw new Error('Invalid storage snapshot name');
          await download(`${qdrant}/collections/${encodeURIComponent(collection)}/snapshots/${encodeURIComponent(name)}`, path.join(staging, 'vectors', `${logical}.snapshot`), qdrantHeaders);
          collections.push(logical);
          await request(`${qdrant}/collections/${encodeURIComponent(collection)}/snapshots/${encodeURIComponent(name)}`, { method: 'DELETE', headers: qdrantHeaders });
        }
        for (const owned of ['artifacts', 'research']) {
          if (existsSync(path.join(dataRoot, owned))) {
            await files(dataRoot, owned); // Reject links to original/user files.
            await cp(path.join(dataRoot, owned), path.join(staging, 'data', owned), { recursive: true, verbatimSymlinks: true });
          }
        }
        const records = [];
        for (const name of await files(staging)) records.push({ path: name, sha256: await sha256(path.join(staging, name)) });
        await writeFile(path.join(staging, 'manifest.json'), JSON.stringify({ format: 1, schemaVersion: 2, created: new Date().toISOString(), namespace: ns, database: db, originalDataRoot: dataRoot, collections, files: records }, null, 2), { mode: 0o600 });
        await verify(staging);
        await rename(staging, snapshot);
        console.log(JSON.stringify({ status: 'ok', snapshot, verifiedFiles: records.length }));
      } catch (error) { await rm(staging, { recursive: true, force: true }); throw error; }
    } else {
      const snapshot = path.resolve(options.snapshot || '');
      const manifest = await verify(snapshot);
      if (!options.database || !options['collection-prefix'] || !options['data-dir']) throw new Error('Restore requires a new database, collection prefix, and data directory');
      if (!/^[a-zA-Z0-9_]+$/.test(options.database) || !/^[a-zA-Z0-9_]+$/.test(options['collection-prefix'])) throw new Error('Invalid restore destination names');
      if (options.database === db || options.database === manifest.database || options['collection-prefix'] === prefix) throw new Error('Restore destinations must differ from the active/original stores');
      const newRoot = path.resolve(options['data-dir']);
      if (existsSync(newRoot)) throw new Error('Restore data directory must not exist');
      const restoreHeaders = { ...surrealHeaders, 'surreal-db': options.database };
      const info = await (await request(`${surreal}/sql`, { method: 'POST', headers: restoreHeaders, body: `USE NS ${ns} DB ${options.database}; INFO FOR DB;` })).json();
      if (info.some(item => item.status !== 'OK') || Object.keys(info.at(-1)?.result?.tables || {}).length) throw new Error('Restore database must be empty');
      const available = (await (await request(`${qdrant}/collections`, { headers: qdrantHeaders })).json()).result.collections.map(item => item.name);
      for (const logical of manifest.collections) if (available.includes(`${options['collection-prefix']}${logical}`)) throw new Error('Restore vector collection already exists');
      let sql = await readFile(path.join(snapshot, 'database.surql'), 'utf8');
      if (Buffer.byteLength(sql, 'utf8') > 128 * 1024 * 1024) throw new Error('Database restore exceeds the 128 MiB import limit');
      // Only self-generated, verified snapshots should be restored: this is a database export, not an untrusted SQL input format.
      const imported = await (await request(`${surreal}/import`, { method: 'POST', headers: restoreHeaders, body: sql })).json();
      if (Array.isArray(imported) && imported.some(item => item.status === 'ERR')) throw new Error('Database import rejected snapshot statements');
      // Rebase owned metadata paths only. Global SQL replacement would corrupt
      // document text that happens to mention the original data directory.
      const paths = `LET $original = ${JSON.stringify(manifest.originalDataRoot)}; LET $destination = ${JSON.stringify(newRoot)}; UPDATE kb_file SET path = string::replace(path, $original, $destination) WHERE string::starts_with(path, $original + '/'); UPDATE search_document SET path = string::replace(path, $original, $destination) WHERE string::starts_with(path, $original + '/');`;
      const rebased = await (await request(`${surreal}/sql`, { method: 'POST', headers: restoreHeaders, body: paths })).json();
      if (rebased.some(item => item.status !== 'OK')) throw new Error('Could not rebase owned restored paths');
      for (const logical of manifest.collections) {
        const form = new FormData();
        const bytes = await readFile(path.join(snapshot, 'vectors', `${logical}.snapshot`));
        if (bytes.length > maximumFile) throw new Error('Vector snapshot is too large');
        form.append('snapshot', new Blob([bytes]), `${logical}.snapshot`);
        await request(`${qdrant}/collections/${encodeURIComponent(options['collection-prefix'] + logical)}/snapshots/upload?priority=snapshot`, { method: 'POST', headers: qdrantHeaders, body: form });
      }
      await mkdir(newRoot, { mode: 0o700 });
      if (existsSync(path.join(snapshot, 'data'))) await cp(path.join(snapshot, 'data'), newRoot, { recursive: true, verbatimSymlinks: true });
      console.log(JSON.stringify({ status: 'ok', restoredDatabase: options.database, collectionPrefix: options['collection-prefix'], dataDirectory: newRoot, activeConfigurationChanged: false }));
    }
  } finally { await guardian.stop(); }
}
