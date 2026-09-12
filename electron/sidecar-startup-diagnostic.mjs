import { createServer } from 'node:http';
import path from 'node:path';
import { diagnosticEnvironment } from './diagnostic-environment.mjs';
import { SidecarManager, developmentSidecarPath } from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const stalled = createServer((_request, _response) => { /* Intentionally never respond. */ });
await new Promise(resolve => stalled.listen(0, '127.0.0.1', resolve));
const manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root,
  env: { ...diagnosticEnvironment(), AYLA_DEVELOPMENT_ROOT: root, SURREALDB_URL: `http://127.0.0.1:${stalled.address().port}` }, startupTimeout: 25000 });
try {
  const start = Date.now();
  await manager.start();
  const startupMilliseconds = Date.now() - start;
  if (await manager.request('check_db_ready')) throw new Error('Stalled storage unexpectedly became ready');
  const capabilities = await manager.request('get_runtime_capabilities');
  if (capabilities.storageReady) throw new Error('Degraded storage capability is incorrect');
  const wav = await manager.request('generate_tts', { text: 'Local speech remains available.', language: 'en' });
  if (!(wav instanceof Uint8Array) || wav.length < 44) throw new Error('Local speech was blocked by unavailable storage');
  await manager.stop();
  console.log(JSON.stringify({ status: 'ok', stalledStorageFallback: true, startupMilliseconds, localSpeechUsable: true, storageReady: false, cleanShutdown: true }, null, 2));
} finally {
  await manager.stop();
  stalled.closeAllConnections();
  await new Promise(resolve => stalled.close(resolve));
}
