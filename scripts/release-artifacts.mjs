import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { cp, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
const output = path.resolve(process.argv[2] || path.join(root, 'release', 'github'));
const { version } = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
await mkdir(output, { recursive: true });
const stage = await mkdtemp(path.join(tmpdir(), 'ayla-artifacts-'));
try {
  const name = `ayla-${version}`;
  await cp(path.join(root, 'release', 'electron', 'linux-unpacked'), path.join(stage, name), { recursive: true, verbatimSymlinks: true });
  const tar = spawnSync('tar', ['--sort=name', '--owner=0', '--group=0', '--numeric-owner', '-czf', path.join(output, `Ayla-${version}-linux-x64.tar.gz`), '-C', stage, name], { stdio: 'inherit' });
  if (tar.status !== 0) throw new Error('Could not create the Arch binary payload');
} finally { await rm(stage, { recursive: true, force: true }); }
// Record the source inputs actually copied into this build. No .env or Git data.
const inputs = [];
async function fingerprint(relative) {
  const entries = await readdir(path.join(root, relative), { withFileTypes: true });
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    const name = path.posix.join(relative, entry.name);
    if (entry.isDirectory()) await fingerprint(name);
    else if (entry.isFile()) {
      const hash = createHash('sha256');
      for await (const chunk of createReadStream(path.join(root, name))) hash.update(chunk);
      inputs.push({ path: name, sha256: hash.digest('hex') });
    } else throw new Error(`Unsupported source input: ${name}`);
  }
}
for (const name of ['crates', 'vendor', 'src', 'styles', 'public', 'electron', 'scripts', 'models', 'tests', 'packaging/licenses', 'tts/supertonic3-int8']) await fingerprint(name);
for (const name of ['Cargo.toml', 'Cargo.lock', 'package.json', 'package-lock.json', 'electron-builder.yml', 'vite.config.js', 'index.html', 'install.sh', 'LICENSE', '.env.example']) {
  const hash = createHash('sha256').update(await readFile(path.join(root, name))).digest('hex');
  inputs.push({ path: name, sha256: hash });
}
await writeFile(path.join(output, 'source-manifest.json'), JSON.stringify({ version, baseCommit: process.env.AYLA_SOURCE_REVISION || null, inputs }, null, 2));
const lines = [];
for (const name of (await readdir(output)).filter(name => /\.(AppImage|tar\.gz|pkg\.tar\.zst|json)$/.test(name)).sort()) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path.join(output, name))) hash.update(chunk);
  lines.push(`${hash.digest('hex')}  ${name}`);
}
await writeFile(path.join(output, 'SHA256SUMS'), `${lines.join('\n')}\n`);
console.log(`Release payloads and checksums written to ${output}. Sign SHA256SUMS with your release key before publication.`);
