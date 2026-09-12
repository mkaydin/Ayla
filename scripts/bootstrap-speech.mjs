import { createHash, randomUUID } from 'node:crypto';
import { createReadStream, createWriteStream, existsSync, lstatSync } from 'node:fs';
import { mkdir, rename, rm } from 'node:fs/promises';
import { pipeline } from 'node:stream/promises';
import { Transform } from 'node:stream';
import { execFileSync, spawn } from 'node:child_process';
import path from 'node:path';
import { readFile } from 'node:fs/promises';

const root = path.resolve(import.meta.dirname, '..');
const directoryIndex = process.argv.indexOf('--directory');
const destination = directoryIndex < 0 ? root : path.resolve(process.argv[directoryIndex + 1]);
const offline = process.argv.includes('--offline') || process.env.AYLA_NATIVE_OFFLINE === '1';
const cache = path.join(root, 'target', 'speech-model-archives');
const manifest = JSON.parse(await readFile(path.join(root, 'models/model-manifest.json')));
async function digest(file) { const hash = createHash('sha256'); for await (const bytes of createReadStream(file)) hash.update(bytes); return hash.digest('hex'); }
function fileLocation(relative) {
  if (path.isAbsolute(relative) || relative.split('/').some(part => part === '..' || !part)) throw new Error('Unsafe model manifest path');
  return path.join(destination, relative);
}
function limit(max, hash) {
  let bytes = 0;
  return new Transform({ transform(chunk, encoding, callback) {
    bytes += chunk.length;
    if (bytes > max) return callback(new Error('Model download exceeds its declared byte limit'));
    hash?.update(chunk); callback(null, chunk);
  } });
}
await mkdir(cache, { recursive: true });
for (const model of manifest.models) {
  const missing = [];
  for (const file of model.files) {
    const location = fileLocation(file.path);
    if (existsSync(location) && lstatSync(location).isSymbolicLink()) throw new Error('Model files may not be symlinks');
    if (!existsSync(location) || lstatSync(location).size !== file.bytes || await digest(location) !== file.sha256) missing.push(file);
  }
  if (!missing.length) { console.log(`Speech model verified: ${model.id}`); continue; }
  const url = new URL(model.source);
  if (url.protocol !== 'https:' || url.hostname !== 'github.com' || !url.pathname.startsWith('/k2-fsa/sherpa-onnx/releases/download/')) throw new Error('Unapproved model archive source');
  const archive = path.join(cache, path.basename(url.pathname));
  if (!existsSync(archive) || (model.archiveSha256 && await digest(archive) !== model.archiveSha256)) {
    if (offline) throw new Error(`Verified speech files/archive missing for ${model.id}; run native:setup online first`);
    const temporary = `${archive}.${randomUUID()}.part`;
    try {
      console.log(`Downloading speech resources: ${model.id}`);
      const response = await fetch(url, { signal: AbortSignal.timeout(600000) });
      if (!response.ok || !response.body) throw new Error(`Speech download HTTP ${response.status}`);
      await pipeline(response.body, limit(1024 * 1024 * 1024), createWriteStream(temporary, { flags: 'wx', mode: 0o600 }));
      if (model.archiveSha256 && await digest(temporary) !== model.archiveSha256) throw new Error('Speech archive checksum mismatch');
      await rename(temporary, archive);
    } finally { await rm(temporary, { force: true }); }
  }
  const members = url.pathname.endsWith('.onnx') ? null : execFileSync('tar', ['-tf', archive], { encoding: 'utf8', maxBuffer: 4 * 1024 * 1024 }).split('\n').filter(Boolean);
  if (members?.some(member => member.startsWith('/') || member.split('/').includes('..') || !/^[a-zA-Z0-9_.+/-]+$/.test(member))) throw new Error('Unsafe speech archive members');
  for (const file of missing) {
    const location = fileLocation(file.path);
    await mkdir(path.dirname(location), { recursive: true });
    let parent = path.dirname(location);
    while (parent !== destination) { if (lstatSync(parent).isSymbolicLink()) throw new Error('Model directories may not be symlinks'); parent = path.dirname(parent); }
    const temporary = `${location}.${randomUUID()}.part`;
    const hash = createHash('sha256');
    let child = null;
    try {
      let input;
      let completed = null;
      if (members) {
        const matches = members.filter(member => !member.endsWith('/') && path.posix.basename(member) === path.basename(file.path));
        if (matches.length !== 1) throw new Error(`Model member is missing or ambiguous: ${file.path}`);
        // Stream one member to a file; never extract archive paths or links.
        child = spawn('tar', ['-xOf', archive, matches[0]], { stdio: ['ignore', 'pipe', 'ignore'] });
        completed = new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', code => code === 0 ? resolve() : reject(new Error('Could not read model archive member'))); });
        input = child.stdout;
      } else input = createReadStream(archive);
      await Promise.all([pipeline(input, limit(file.bytes, hash), createWriteStream(temporary, { flags: 'wx', mode: 0o644 })), completed]);
      if (lstatSync(temporary).size !== file.bytes || hash.digest('hex') !== file.sha256) throw new Error(`Speech model checksum mismatch: ${file.path}`);
      await rename(temporary, location);
    } finally { child?.kill(); await rm(temporary, { force: true }); }
  }
  console.log(`Speech model installed and verified: ${model.id}`);
}
