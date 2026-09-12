import { appendFileSync, chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { join } from 'node:path';

const MAX_LOG_BYTES = 1024 * 1024;
const STATES = new Set(['starting', 'ready', 'restarting', 'failed', 'stopped', 'stopping', 'crashed']);
const EVENTS = new Set(['application-start', 'application-stop', 'backend-status', 'renderer-crash']);

// Allow fields individually. Never retain exception messages, stderr, URLs,
// settings, document names/text, prompts, or audio in support diagnostics.
export function sanitizeDiagnostic(record) {
  if (!record || !EVENTS.has(record.event)) return null;
  const result = { timestamp: /^\d{4}-\d{2}-\d{2}T[\d:.]+Z$/.test(record.timestamp) ? record.timestamp : new Date().toISOString(), event: record.event };
  if (STATES.has(record.state)) result.state = record.state;
  for (const field of ['restartAttempt', 'protocolVersion', 'exitCode']) {
    if (Number.isSafeInteger(record[field]) && Math.abs(record[field]) <= 100000) result[field] = record[field];
  }
  for (const field of ['appVersion', 'electronVersion', 'nodeVersion', 'backendVersion']) {
    if (typeof record[field] === 'string' && /^\d+(?:\.\d+){1,3}(?:[-+][a-zA-Z0-9.]+)?$/.test(record[field])) result[field] = record[field];
  }
  return result;
}

export function writeDiagnostic(dataRoot, record, maxBytes = MAX_LOG_BYTES) {
  try {
    const directory = join(dataRoot, 'logs');
    mkdirSync(directory, { recursive: true, mode: 0o700 });
    if (lstatSync(directory).isSymbolicLink()) return;
    chmodSync(directory, 0o700);
    const file = join(directory, 'runtime-events.jsonl');
    const safe = sanitizeDiagnostic({ ...record, timestamp: new Date().toISOString() });
    if (!safe) return;
    const line = JSON.stringify(safe) + '\n';
    for (const name of [file, `${file}.1`, `${file}.2`]) if (existsSync(name) && !lstatSync(name).isFile()) return;
    if (existsSync(file) && lstatSync(file).size + Buffer.byteLength(line) > maxBytes) {
      rmSync(`${file}.2`, { force: true });
      if (existsSync(`${file}.1`)) renameSync(`${file}.1`, `${file}.2`);
      renameSync(file, `${file}.1`);
    }
    appendFileSync(file, line, { mode: 0o600 });
    chmodSync(file, 0o600);
  } catch {
    // Diagnostics must never prevent startup or a clean shutdown.
  }
}

export function collectSupportDiagnostics(dataRoot) {
  const records = [];
  for (const suffix of ['.2', '.1', '']) {
    const file = join(dataRoot, 'logs', `runtime-events.jsonl${suffix}`);
    if (!existsSync(file)) continue;
    const info = lstatSync(file);
    if (!info.isFile() || info.size > MAX_LOG_BYTES) continue;
    for (const line of readFileSync(file, 'utf8').split('\n')) {
      if (!line || line.length > 4096) continue;
      try { const safe = sanitizeDiagnostic(JSON.parse(line)); if (safe) records.push(safe); } catch { /* Incomplete or invalid record. */ }
    }
  }
  return { schemaVersion: 1, platform: process.platform, architecture: process.arch, generatedAt: new Date().toISOString(), runtimeEvents: records.slice(-10000) };
}
