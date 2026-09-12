import { diagnosticEnvironment, backendCommandCatalog } from './diagnostic-environment.mjs';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { developmentSidecarPath, SidecarManager } from './sidecar-manager.mjs';

const fixture = process.argv[2];
if (!fixture) {
  throw new Error('Usage: node electron/sidecar-stt-diagnostic.mjs FILE.pcm');
}

const root = path.resolve(import.meta.dirname, '..');
const manager = new SidecarManager({
  binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root),
  args: ['--offline'],
  cwd: root,
  env: {
    ...diagnosticEnvironment(),
    AYLA_DEVELOPMENT_ROOT: root,
  },
  startupTimeout: 60_000,
  restartLimit: 0,
});

const finals = [];
const partials = [];
manager.on('backend-event', message => {
  if (message.name === 'stt-final') finals.push(message.payload?.text || '');
  if (message.name === 'stt-partial') partials.push(message.payload?.text || '');
});

try {
  await manager.start();
  const capabilities = await manager.request('get_runtime_capabilities');
  if (!capabilities?.stt?.available) {
    throw new Error(`STT unavailable: ${capabilities?.stt?.reason || 'unknown reason'}`);
  }
  const { sessionId } = await manager.request('start_stt');
  const pcm = readFileSync(path.resolve(fixture));
  for (let offset = 0; offset < pcm.length; offset += 3_200) {
    await manager.request('stt_push_audio', {
      sessionId,
      pcm: pcm.subarray(offset, Math.min(offset + 3_200, pcm.length)),
    });
  }
  const silence = Buffer.alloc(32_000);
  for (let offset = 0; offset < silence.length; offset += 3_200) {
    await manager.request('stt_push_audio', {
      sessionId,
      pcm: silence.subarray(offset, offset + 3_200),
    });
  }
  await manager.request('stop_stt', { sessionId });
  if (!finals.some(Boolean)) throw new Error('STT sidecar emitted no final transcript');
  console.log(JSON.stringify({
    status: 'ok',
    model: capabilities.stt.model,
    provider: capabilities.stt.provider,
    partials: partials.filter(Boolean).length,
    finals: finals.filter(Boolean),
  }, null, 2));
} finally {
  await manager.stop().catch(() => {});
}
