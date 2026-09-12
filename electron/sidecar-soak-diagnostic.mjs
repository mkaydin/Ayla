import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { diagnosticEnvironment } from './diagnostic-environment.mjs';
import { SidecarManager, developmentSidecarPath } from './sidecar-manager.mjs';
const root = path.resolve(import.meta.dirname, '..');
const minutes = Number(process.argv[2] || 30);
if (!Number.isInteger(minutes) || minutes < 1 || minutes > 60) throw new Error('Use an integer duration of 1–60 minutes');
const env = { ...diagnosticEnvironment(), AYLA_DEVELOPMENT_ROOT: root, AYLA_OLLAMA_LONG_TIMEOUT_SECS: '180' };
const manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root, env, startupTimeout: 60000 });
const name = 'soak-public-fixture.md';
const text = 'SoakSensor measures pressure. Its calibrated pressure is 42.75 MPa.';
const samples = [];
let cycles = 0;
let wavs = 0;
try {
  const hello = await manager.start();
  if (!await manager.request('check_db_ready')) throw new Error('Isolated storage is unavailable');
  const started = Date.now();
  while (Date.now() - started < minutes * 60000) {
    if (cycles % 6 === 0) {
      await manager.request('commit_document', { name, path: 'fixture://soak', chars: text.length, sourceType: 'diagnostic', text });
      const context = await manager.request('enrich_prompt', { userQuery: 'SoakSensor calibrated pressure', basePrompt: 'You are Ayla.' });
      if (!String(context).includes('42.75')) throw new Error('Knowledge stopped being retrievable');
      const wav = await manager.request('generate_tts', { text: 'Ayla runtime verification.', language: 'en' });
      if (!(wav instanceof Uint8Array) || wav.length < 44) throw new Error('Speech failed during mixed workload');
      wavs++;
      await manager.request('delete_file_record', { name, deleteNodes: true });
    }
    await manager.request('get_runtime_capabilities');
    const graph = await manager.request('get_graph_v2_data');
    if (graph.nodes.some(node => node.sources.includes(name))) throw new Error('Deleted source returned during soak');
    if (manager.pending.size) throw new Error('Completed work left pending protocol requests');
    const status = readFileSync(`/proc/${hello.pid}/status`, 'utf8');
    const rssKiB = Number(status.match(/^VmRSS:\s+(\d+)/m)?.[1]);
    const descriptors = readdirSync(`/proc/${hello.pid}/fd`).length;
    samples.push({ elapsedSeconds: Math.round((Date.now() - started) / 1000), rssKiB, descriptors });
    cycles++;
    if (cycles % 30 === 0) console.error(JSON.stringify({ progressMinutes: Math.round((Date.now() - started) / 60000), cycles, rssKiB, descriptors }));
    await new Promise(resolve => setTimeout(resolve, 10000));
  }
  const warmed = samples.filter(sample => sample.elapsedSeconds >= Math.min(300, minutes * 30));
  const baseline = warmed[0] || samples[0];
  const last = samples.at(-1);
  if (last.rssKiB > baseline.rssKiB * 1.5 + 100 * 1024) throw new Error('Backend RSS exceeded the post-warmup growth tolerance');
  if (last.descriptors > baseline.descriptors + 32) throw new Error('Backend file descriptors grew without bound');
  await manager.stop();
  console.log(JSON.stringify({ status: 'ok', durationMinutes: minutes, cycles, speechRuns: wavs, postWarmupBaseline: baseline, final: last, peakRssKiB: Math.max(...samples.map(sample => sample.rssKiB)), noPendingRequests: true, cleanShutdown: true, scope: 'Automated backend IPC, storage/retrieval/deletion, and speech; desktop FPS and real microphone acceptance are separate' }, null, 2));
} finally { await manager.stop(); }
