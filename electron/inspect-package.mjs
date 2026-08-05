import { existsSync, statSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';

const root = path.resolve(import.meta.dirname, '..');
const resources = path.join(root, 'release', 'electron', 'linux-unpacked', 'resources');
const required = [
  ['Rust sidecar', path.join(resources, 'bin', 'ayla-sidecar'), true],
  ['Piper model', path.join(resources, 'tts', 'tts_piper_amy_low', 'en_US-amy-low.onnx'), false],
  ['Electron application archive', path.join(resources, 'app.asar'), false],
];

const failures = [];
const result = {};
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

if (failures.length) {
  throw new Error(`Electron package inspection failed:\n- ${failures.join('\n- ')}`);
}

console.log(JSON.stringify({ status: 'ok', resources: result }, null, 2));
