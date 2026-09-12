import { createPackage } from '@electron/asar';
import { cp, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { stageModelResources } from '../electron/stage-resources.mjs';
import { stageDependencyLicenses } from '../electron/dependency-licenses.mjs';

const root = path.resolve(import.meta.dirname, '..');
const output = path.resolve(process.argv[2] || path.join(root, 'release', 'arch-payload'));
const stage = await mkdtemp(path.join(tmpdir(), 'ayla-arch-source-'));
try {
  await mkdir(output, { recursive: true });
  for (const name of ['dist', 'package.json', 'LICENSE', 'node_modules/@msgpack/msgpack']) {
    const destination = path.join(stage, name);
    await mkdir(path.dirname(destination), { recursive: true });
    await cp(path.join(root, name), destination, { recursive: true, verbatimSymlinks: true });
  }
  await mkdir(path.join(stage, 'electron'), { recursive: true });
  for (const name of ['main.mjs', 'ipc-policy.mjs', 'preload.cjs', 'sidecar-manager.mjs', 'sidecar-protocol.mjs', 'static-server.mjs', 'storage-recovery.mjs', 'support-diagnostics.mjs']) {
    await cp(path.join(root, 'electron', name), path.join(stage, 'electron', name));
  }
  await createPackage(stage, path.join(output, 'app.asar'));
  stageModelResources(path.join(root, '.electron-resources'));
  for (const [from, to] of [
    ['target/release/ayla-sidecar', 'bin/ayla-sidecar'],
    ['target/native/pdfium/lib/libpdfium.so', 'lib/libpdfium.so'],
    ['target/native/pdfium/LICENSE', 'licenses/PDFIUM_LICENSE'],
    ['target/native/pdfium/licenses', 'licenses/pdfium'],
    ['packaging/licenses', 'licenses'],
    ['LICENSE', 'licenses/AYLA_LICENSE'],
    ['vendor/sherpa-onnx/sherpa-onnx/LICENSE', 'licenses/SHERPA_ONNX_LICENSE'],
    ['electron/assets/icon.png', 'assets/icon.png'],
    ['.electron-resources/models/model-manifest.json', 'models/model-manifest.json'],
    ['.electron-resources/models/stt', 'models/stt'],
    ['.electron-resources/tts/supertonic3-int8', 'tts/supertonic3-int8'],
  ]) {
    await mkdir(path.dirname(path.join(output, to)), { recursive: true });
    await cp(path.join(root, from), path.join(output, to), { recursive: true, verbatimSymlinks: true });
  }
  stageDependencyLicenses(path.join(output, 'licenses'));
} finally { await rm(stage, { recursive: true, force: true }); }
console.log(`Arch application payload created at ${output}; use the system Electron 41 launcher.`);
