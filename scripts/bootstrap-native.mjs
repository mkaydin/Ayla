import { createHash, randomUUID } from 'node:crypto';
import { createReadStream, createWriteStream, existsSync } from 'node:fs';
import { mkdir, rename, rm } from 'node:fs/promises';
import { pipeline } from 'node:stream/promises';
import { Transform } from 'node:stream';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '..');
const offline = process.argv.includes('--offline') || process.env.AYLA_NATIVE_OFFLINE === '1';
const directory = path.join(root, 'target', 'sherpa-onnx-prebuilt');
const filename = 'sherpa-onnx-v1.13.4-linux-x64-static-lib.tar.bz2';
const digest = '98b0e31996426f6e78244dbce1955548f2c64e8f01c4be75b85af7cdaa2e8d5c';
const archive = path.join(directory, filename);
function boundedDownload() {
  let bytes = 0;
  return new Transform({ transform(chunk, encoding, callback) {
    bytes += chunk.length;
    callback(bytes > 1024 * 1024 * 1024 ? new Error('Native archive exceeds the 1 GiB download limit') : null, chunk);
  } });
}
async function sha256(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

if (process.platform !== 'linux' || process.arch !== 'x64') {
  throw new Error('The first release supports Linux x86_64 only');
}
await mkdir(directory, { recursive: true });

const pdfiumArchive = path.join(directory, 'pdfium-linux-x64-7881.tgz');
const pdfiumDigest = '1470e21b8b4a3b4ad7f85684e2da11d94f3b69a86d81dee11b9b6709d927ac1d';
if (!existsSync(pdfiumArchive) || await sha256(pdfiumArchive) !== pdfiumDigest) {
  if (offline) throw new Error('Verified PDFium archive is missing; run native:setup online first');
  const temporary = `${pdfiumArchive}.${randomUUID()}.part`;
  try {
    const response = await fetch('https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7881/pdfium-linux-x64.tgz', { signal: AbortSignal.timeout(600_000) });
    if (!response.ok || !response.body) throw new Error(`PDFium download returned HTTP ${response.status}`);
    await pipeline(response.body, boundedDownload(), createWriteStream(temporary, { flags: 'wx', mode: 0o600 }));
    if (await sha256(temporary) !== pdfiumDigest) throw new Error('PDFium archive checksum mismatch');
    await rename(temporary, pdfiumArchive);
  } finally { await rm(temporary, { force: true }); }
}
const pdfiumDirectory = path.join(root, 'target', 'native', 'pdfium');
await mkdir(pdfiumDirectory, { recursive: true });
const extraction = spawnSync('tar', ['-xzf', pdfiumArchive, '-C', pdfiumDirectory], { stdio: 'inherit' });
if (extraction.status !== 0) throw new Error('Verified PDFium archive could not be extracted');
console.log('PDFium 7881 native library and licenses verified');

if (existsSync(archive) && await sha256(archive) === digest) {
  console.log('Sherpa-ONNX 1.13.4 native archive verified');
} else {
  if (offline) throw new Error('Verified native archive is missing; run bootstrap-native.mjs online first');
  const temporary = `${archive}.${randomUUID()}.part`;
  try {
    const response = await fetch(`https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.13.4/${filename}`, { signal: AbortSignal.timeout(600_000) });
    if (!response.ok || !response.body) throw new Error(`Native archive download returned HTTP ${response.status}`);
    await pipeline(response.body, boundedDownload(), createWriteStream(temporary, { flags: 'wx', mode: 0o600 }));
    if (await sha256(temporary) !== digest) throw new Error('Native archive checksum mismatch');
    await rename(temporary, archive);
    console.log('Sherpa-ONNX 1.13.4 native archive downloaded and verified');
  } finally {
    await rm(temporary, { force: true });
  }
}
