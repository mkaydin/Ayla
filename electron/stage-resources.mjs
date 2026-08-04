import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  statSync,
} from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const electronDirectory = path.dirname(fileURLToPath(import.meta.url));
const projectDirectory = path.dirname(electronDirectory);
const executableSuffix = process.platform === 'win32' ? '.exe' : '';

export const DEFAULT_STAGE_DIRECTORY = path.join(projectDirectory, '.electron-resources');

export function resourceManifest({
  root = projectDirectory,
  env = process.env,
  stageDirectory = DEFAULT_STAGE_DIRECTORY,
} = {}) {
  return [
    {
      name: 'Rust sidecar',
      source: env.ALICE_SIDECAR_BIN
        || path.join(root, 'crates', 'alice-sidecar', 'target', 'release', `alice-sidecar${executableSuffix}`),
      destination: path.join(stageDirectory, 'bin', `alice-sidecar${executableSuffix}`),
      buildHint: 'npm run sidecar:build:release',
    },
    {
      name: 'OCR helper',
      source: env.ALICE_OCR_BIN
        || path.resolve(root, '..', 'ocr_helper_system', 'target', 'release', `ocr-helper${executableSuffix}`),
      destination: path.join(stageDirectory, 'tools', `ocr-helper${executableSuffix}`),
      buildHint: '(cd ../ocr_helper_system && cargo build --release)',
    },
    {
      name: 'Meta Search helper',
      source: env.ALICE_SEARCH_BIN
        || path.resolve(root, '..', 'meta_search_Js_render', 'target', 'release', `meta_search${executableSuffix}`),
      destination: path.join(stageDirectory, 'tools', `meta_search${executableSuffix}`),
      buildHint: '(cd ../meta_search_Js_render && cargo build --release)',
    },
  ];
}

export function stageResources(options = {}) {
  const manifest = resourceManifest(options);
  const missing = manifest.filter(item => {
    if (!existsSync(item.source)) return true;
    try {
      return !statSync(item.source).isFile();
    } catch {
      return true;
    }
  });
  if (missing.length) {
    const details = missing
      .map(item => `- ${item.name}: ${item.source}\n  Build with: ${item.buildHint}`)
      .join('\n');
    throw new Error(`Cannot stage required Electron resources:\n${details}`);
  }

  for (const item of manifest) {
    mkdirSync(path.dirname(item.destination), { recursive: true });
    copyFileSync(item.source, item.destination);
    if (process.platform !== 'win32') chmodSync(item.destination, 0o755);
  }
  return manifest;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const manifest = stageResources();
    for (const item of manifest) {
      console.log(`Staged ${item.name}: ${item.destination}`);
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
