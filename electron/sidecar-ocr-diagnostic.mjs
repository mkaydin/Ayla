import {
  readdirSync,
  statSync,
} from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import {
  developmentSidecarPath,
  SidecarManager,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');

function listPdfs(directory) {
  const result = [];
  let entries = [];
  try {
    entries = readdirSync(directory, { withFileTypes: true });
  } catch {
    return result;
  }
  for (const entry of entries) {
    const filePath = path.join(directory, entry.name);
    if (entry.isDirectory()) result.push(...listPdfs(filePath));
    else if (entry.isFile() && entry.name.toLowerCase().endsWith('.pdf')) result.push(filePath);
  }
  return result;
}

function selectFixture() {
  if (process.env.AYLA_OCR_FIXTURE) return path.resolve(process.env.AYLA_OCR_FIXTURE);
  const dataRoot = process.env.AYLA_DATA_DIR || path.join(root, 'ayla-data');
  const downloads = path.join(dataRoot, 'research', 'downloads');
  const candidates = listPdfs(downloads)
    .map(filePath => ({ filePath, bytes: statSync(filePath).size }))
    .sort((left, right) => left.bytes - right.bytes);
  if (!candidates.length) {
    throw new Error(
      'No OCR fixture found. Set AYLA_OCR_FIXTURE to a local PDF or run a paper search first.',
    );
  }
  return candidates[0].filePath;
}

function temporaryOcrOutputs() {
  return new Set(
    readdirSync('/tmp')
      .filter(name => /^ayla-ocr-/.test(name))
      .map(name => path.join('/tmp', name)),
  );
}

const fixture = selectFixture();
const outputsBefore = temporaryOcrOutputs();
const manager = new SidecarManager({
  binaryPath: developmentSidecarPath(root),
  cwd: root,
  env: {
    ...process.env,
    AYLA_DATA_DIR: process.env.AYLA_DATA_DIR || path.join(root, 'ayla-data'),
    AYLA_DEVELOPMENT_ROOT: path.join(root, 'src-tauri'),
  },
  startupTimeout: 60_000,
  restartLimit: 0,
});
let started = null;

try {
  const hello = await manager.start();
  manager.on('backend-event', message => {
    if (message.name === 'ocr-started') {
      started = {
        operationId: message.operation_id,
        pid: message.payload?.pid,
      };
    }
  });
  const ocr = await manager.request('run_ocr', { filePath: fixture });
  if (!ocr?.text?.trim() || Number(ocr.chars || 0) <= 0) {
    throw new Error(`OCR returned no text: ${JSON.stringify(ocr)}`);
  }
  const leakedOutputs = [...temporaryOcrOutputs()]
    .filter(filePath => !outputsBefore.has(filePath));
  if (leakedOutputs.length) {
    throw new Error(`OCR left temporary output files: ${leakedOutputs.join(', ')}`);
  }
  await manager.stop();
  console.log(JSON.stringify({
    status: 'ok',
    protocolVersion: hello.protocolVersion,
    backendVersion: hello.backendVersion,
    fixture: path.basename(fixture),
    fixtureBytes: statSync(fixture).size,
    chars: ocr.chars,
    pages: ocr.pages,
    processEvent: started,
    temporaryOutputsRemaining: leakedOutputs.length,
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  await manager.stop().catch(() => {});
  throw error;
}
