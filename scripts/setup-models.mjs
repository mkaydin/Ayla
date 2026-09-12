import { spawnSync } from 'node:child_process';

const endpoint = process.env.OLLAMA_URL || 'http://127.0.0.1:11434';
const response = await fetch(`${endpoint.replace(/\/$/, '')}/api/tags`, { signal: AbortSignal.timeout(10_000) });
if (!response.ok) throw new Error(`Ollama is unavailable: HTTP ${response.status}`);
const { models = [] } = await response.json();
const installed = new Set(models.map(model => model.name));
const required = [
  process.env.AYLA_OCR_MODEL || 'glm-ocr:latest',
  process.env.AYLA_EMBED_MODEL || 'qwen3-embedding:0.6b',
  'gemma4:e4b',
];
if (process.argv.includes('--cloud')) required.push('nemotron-3-super:cloud');
for (const model of new Set(required)) {
  if (installed.has(model) && !process.argv.includes('--refresh')) {
    console.log(`Installed: ${model}`);
    continue;
  }
  const result = spawnSync('ollama', ['pull', model], { stdio: 'inherit', env: { ...process.env, OLLAMA_HOST: endpoint } });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Ollama pull failed for ${model}`);
}
console.log('Required local models are ready. Cloud use still requires the application consent settings and Ollama authorization.');
