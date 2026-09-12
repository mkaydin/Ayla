import { createHash } from 'node:crypto';
import { existsSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { listPackage } from '@electron/asar';
import { spawnSync } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '..');
const resources = path.join(root, 'release', 'electron', 'linux-unpacked', 'resources');
const required = [
  ['Rust sidecar', path.join(resources, 'bin', 'ayla-sidecar'), true],
  ['Supertonic duration model', path.join(resources, 'tts', 'supertonic3-int8', 'duration_predictor.int8.onnx'), false],
  ['Supertonic text encoder', path.join(resources, 'tts', 'supertonic3-int8', 'text_encoder.int8.onnx'), false],
  ['Supertonic vector estimator', path.join(resources, 'tts', 'supertonic3-int8', 'vector_estimator.int8.onnx'), false],
  ['Supertonic vocoder', path.join(resources, 'tts', 'supertonic3-int8', 'vocoder.int8.onnx'), false],
  ['Supertonic model license', path.join(resources, 'tts', 'supertonic3-int8', 'MODEL_LICENSE'), false],
  ['Omnilingual STT model', path.join(resources, 'models', 'stt', 'sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-int8-2025-11-12', 'model.int8.onnx'), false],
  ['Silero VAD model', path.join(resources, 'models', 'stt', 'silero_vad.onnx'), false],
  ['Silero VAD license', path.join(resources, 'models', 'stt', 'SILERO_VAD_LICENSE'), false],
  ['Electron application archive', path.join(resources, 'app.asar'), false],
];

const failures = [];
const result = {};
const dependencyInventory = path.join(resources, 'licenses/DEPENDENCY_LICENSES.json');
if (!existsSync(dependencyInventory)) failures.push('Dependency license inventory is missing');
else {
  const inventory = JSON.parse(readFileSync(dependencyInventory, 'utf8'));
  for (const pkg of inventory.packages || []) for (const notice of pkg.notices || []) {
    const location = path.join(resources, 'licenses', notice.path);
    if (!existsSync(location) || createHash('sha256').update(readFileSync(location)).digest('hex') !== notice.sha256) failures.push(`Dependency notice missing or changed: ${pkg.name}`);
  }
  result['Dependency notices'] = { packages: inventory.packages?.length || 0 };
}
for (const [name, filePath, executable] of required) {
  if (!existsSync(filePath) || !statSync(filePath).isFile()) {
    failures.push(`${name} is missing: ${filePath}`);
    continue;
  }
  const stat = statSync(filePath);
  if (executable && process.platform !== 'win32' && (stat.mode & 0o111) === 0) {
    failures.push(`${name} is not executable: ${filePath}`);
  }
  result[name] = {
    path: path.relative(root, filePath),
    bytes: stat.size,
    executableRequired: executable,
    executable: executable
      ? process.platform === 'win32' || (stat.mode & 0o111) !== 0
      : null,
  };
}

const archivePath = path.join(resources, 'app.asar');
if (existsSync(archivePath)) {
  const files = listPackage(archivePath);
  if (!files.includes('/node_modules/@msgpack/msgpack/package.json')) failures.push('MessagePack runtime dependency is missing from app.asar');
  if (files.some(file => /(?:^|\/)Mk6(?:_1\.0)?\//.test(file))) failures.push('Optional licensed avatar assets were included in the release');
  if (files.some(file => file.includes('@tauri-apps') || file.startsWith('/src-tauri/'))) failures.push('Obsolete desktop host was included in the release');
  result['Runtime imports'] = { messagepackPresent: files.includes('/node_modules/@msgpack/msgpack/package.json') };
}
const binary = path.join(resources, 'bin', 'ayla-sidecar');
if (existsSync(binary)) {
  const protocol = spawnSync(binary, ['--protocol-version'], { encoding: 'utf8', timeout: 10000 });
  if (protocol.status !== 0 || protocol.stdout.trim() !== '1') failures.push(`Sidecar cannot start outside the checkout: ${protocol.stderr || protocol.error || protocol.stdout}`);
}

const modelManifestPath = path.join(resources, 'models', 'model-manifest.json');
if (!existsSync(modelManifestPath)) {
  failures.push(`Model manifest is missing: ${modelManifestPath}`);
} else {
  const manifest = JSON.parse(readFileSync(modelManifestPath, 'utf8'));
  for (const model of manifest.models || []) {
    for (const file of model.files || []) {
      const filePath = path.join(resources, file.path);
      if (!existsSync(filePath)) {
        failures.push(`${model.id} file is missing: ${filePath}`);
        continue;
      }
      const metadata = statSync(filePath);
      if (process.platform !== 'win32' && !(metadata.mode & 0o004)) failures.push(`${model.id} is not readable by users other than the package builder: ${file.path}`);
      const bytes = metadata.size;
      const sha256 = createHash('sha256').update(readFileSync(filePath)).digest('hex');
      if (bytes !== file.bytes) {
        failures.push(`${model.id} size mismatch for ${file.path}: ${bytes} != ${file.bytes}`);
      }
      if (sha256 !== file.sha256) {
        failures.push(`${model.id} hash mismatch for ${file.path}`);
      }
    }
    const licensePath = path.join(resources, model.licenseFile || '');
    if (!model.licenseFile || !existsSync(licensePath)) {
      failures.push(`${model.id} license is missing: ${licensePath}`);
    }
  }
  result['Model manifest'] = {
    path: path.relative(root, modelManifestPath),
    models: manifest.models?.length || 0,
    verified: failures.length === 0,
  };
}

if (failures.length) {
  throw new Error(`Electron package inspection failed:\n- ${failures.join('\n- ')}`);
}

console.log(JSON.stringify({ status: 'ok', resources: result }, null, 2));
