import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  cpSync,
  readFileSync,
  writeFileSync,
  statSync,
  rmSync,
  readdirSync,
  lstatSync,
} from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { stageDependencyLicenses } from './dependency-licenses.mjs';

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
      source: env.AYLA_SIDECAR_BIN
        || path.join(root, 'target', 'release', `ayla-sidecar${executableSuffix}`),
      destination: path.join(stageDirectory, 'bin', `ayla-sidecar${executableSuffix}`),
      buildHint: 'npm run sidecar:build:release',
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

export function stageModelResources(directory = DEFAULT_STAGE_DIRECTORY) {
  for (const relative of ['models/stt', 'tts/supertonic3-int8']) {
    const destination = path.join(directory, relative);
    rmSync(destination, { recursive: true, force: true });
    mkdirSync(path.dirname(destination), { recursive: true });
    cpSync(path.join(projectDirectory, relative), destination, { recursive: true, verbatimSymlinks: true });
    function normalize(location) {
      const entry = lstatSync(location);
      if (entry.isSymbolicLink()) throw new Error('Bundled model resources may not be symlinks');
      chmodSync(location, entry.isDirectory() ? 0o755 : 0o644);
      if (entry.isDirectory()) for (const name of readdirSync(location)) normalize(path.join(location, name));
    }
    normalize(destination);
  }
  copyFileSync(path.join(projectDirectory, 'models/model-manifest.json'), path.join(directory, 'models/model-manifest.json'));
  chmodSync(path.join(directory, 'models/model-manifest.json'), 0o644);
}

export function stageNativeRuntime(binary, directory = DEFAULT_STAGE_DIRECTORY) {
  const libDirectory = path.join(directory, 'lib');
  rmSync(libDirectory, { recursive: true, force: true });
  mkdirSync(libDirectory, { recursive: true });
  const pdfium = path.join(projectDirectory, 'target', 'native', 'pdfium');
  const library = path.join(pdfium, 'lib', 'libpdfium.so');
  if (!existsSync(library)) throw new Error('PDFium is missing; run npm run native:setup');
  copyFileSync(library, path.join(libDirectory, 'libpdfium.so'));
  if (existsSync(path.join(pdfium, 'licenses'))) cpSync(path.join(pdfium, 'licenses'), path.join(directory, 'licenses', 'pdfium'), { recursive: true });
  if (existsSync(path.join(projectDirectory, 'packaging', 'licenses'))) cpSync(path.join(projectDirectory, 'packaging', 'licenses'), path.join(directory, 'licenses'), { recursive: true });
  const notices = path.join(directory, 'licenses', 'native');
  mkdirSync(notices, { recursive: true });
  if (existsSync(path.join(pdfium, 'LICENSE'))) copyFileSync(path.join(pdfium, 'LICENSE'), path.join(directory, 'licenses', 'PDFIUM_LICENSE'));
  for (const name of ['libssl3', 'libstdc++6', 'libgcc-s1', 'liblzma5', 'libbz2-1.0', 'libzstd1', 'libbrotli1']) {
    const copyright = path.join('/usr/share/doc', name, 'copyright');
    if (existsSync(copyright)) copyFileSync(copyright, path.join(notices, name + '.copyright'));
  }
  const excluded = /^(?:libc\.so|libm\.so|libdl\.so|libpthread\.so|librt\.so|ld-linux)/;
  const libraries = new Map();
  for (const executable of [binary, library]) {
    const output = execFileSync('ldd', [executable], { encoding: 'utf8' });
    if (output.includes('not found')) throw new Error(`Unresolved native dependency: ${output}`);
    for (const match of output.matchAll(/^\s*(\S+) => (\/\S+)/gm)) {
      const name = path.basename(match[1]);
      if (!excluded.test(name)) libraries.set(name, match[2]);
    }
  }
  const records = [];
  for (const [name, source] of libraries) {
    if (name === 'libpdfium.so') continue;
    copyFileSync(source, path.join(libDirectory, name));
    records.push({ name, sha256: createHash('sha256').update(readFileSync(source)).digest('hex') });
  }
  records.push({ name: 'libpdfium.so', sha256: createHash('sha256').update(readFileSync(library)).digest('hex') });
  const versions = [binary, library, ...libraries.values()].flatMap(file =>
    execFileSync('readelf', ['--version-info', file], { encoding: 'utf8' }).match(/GLIBC_\d+\.\d+/g) || []);
  const minimumGlibc = [...new Set(versions)].sort((a, b) => {
    const av = a.slice(6).split('.').map(Number), bv = b.slice(6).split('.').map(Number);
    return av[0] - bv[0] || av[1] - bv[1];
  }).at(-1)?.slice(6);
  if (process.env.AYLA_PORTABLE_RELEASE === '1' && Number(minimumGlibc?.split('.')[1]) > 36) {
    throw new Error(`Portable release requires glibc <= 2.36; this sidecar requires ${minimumGlibc}. Use the release container.`);
  }
  writeFileSync(path.join(directory, 'native-manifest.json'), JSON.stringify({ platform: 'linux-x64', minimumGlibc, libraries: records }, null, 2));
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const manifest = stageResources();
    if (process.platform === 'linux') stageNativeRuntime(manifest[0].destination);
    stageModelResources();
    stageDependencyLicenses(path.join(DEFAULT_STAGE_DIRECTORY, 'licenses'));
    for (const item of manifest) {
      console.log(`Staged ${item.name}: ${item.destination}`);
    }
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
