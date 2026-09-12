import { existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
let failures = 0;
for (const [binary, args] of [['node', ['--version']], ['npm', ['--version']], ['cargo', ['--version']], ['ollama', ['--version']]]) {
  const result = spawnSync(binary, args, { encoding: 'utf8' });
  if (result.error || result.status !== 0) { console.log(`MISSING: ${binary}`); failures++; }
  else console.log(`${binary}: ${(result.stdout || result.stderr).trim().split('\n')[0]}`);
}
for (const relative of ['vendor/sherpa-onnx/sherpa-onnx/Cargo.toml', 'models/model-manifest.json', 'crates/ayla-sidecar/Cargo.toml']) {
  if (!existsSync(path.join(root, relative))) { console.log(`MISSING: ${relative}`); failures++; }
}
const manifest = JSON.parse(readFileSync(path.join(root, 'models/model-manifest.json'), 'utf8'));
for (const model of manifest.models) {
  const missing = model.files.filter(file => !existsSync(path.join(root, file.path)));
  console.log(`${model.id}: ${missing.length ? 'incomplete' : 'files present'}`);
  if (missing.length) failures++;
}
for (const [name, url] of [
  ['Ollama', process.env.OLLAMA_URL || 'http://127.0.0.1:11434'],
  ['SurrealDB', process.env.SURREALDB_URL || 'http://127.0.0.1:8000/health'],
  ['Qdrant', 'http://127.0.0.1:6333/'],
  ['SearXNG', process.env.SEARXNG_URL || 'http://127.0.0.1:8888/healthz'],
  ['Lightpanda', 'http://127.0.0.1:9222/json/version'],
]) {
  try { const response = await fetch(url, { signal: AbortSignal.timeout(3000) }); console.log(`${name}: HTTP ${response.status}`); }
  catch { console.log(`${name}: unavailable (dependent features will need setup)`); }
}
console.log('Primary runtime: Electron + Rust. Avatar assets are optional. Run npm run models:setup for local model setup.');
process.exitCode = failures ? 1 : 0;
