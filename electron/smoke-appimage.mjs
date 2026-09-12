import { mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const directory = path.resolve(process.argv[2] || 'release/electron');
const file = readdirSync(directory).find(name => name.endsWith('.AppImage'));
if (!file) throw new Error('No AppImage to verify');
const root = mkdtempSync(path.join(tmpdir(), 'ayla-appimage-smoke-'));
try {
  const result = spawnSync('xvfb-run', ['-a', path.join(directory, file), '--appimage-extract-and-run', '--smoke-test', '--disable-gpu'], {
    env: { ...process.env, AYLA_DATA_DIR: root, AYLA_DISABLE_AVATAR: '1' }, encoding: 'utf8', timeout: 180000, maxBuffer: 8 * 1024 * 1024,
  });
  const report = (result.stdout || '').split('\n').map(line => { try { return JSON.parse(line); } catch { return null; } }).find(record => record?.status === 'ok' && record.backend?.backendAvailable);
  if (result.status !== 0 || !report || ['chat', 'ocr', 'code', 'search', 'kb', 'status'].some(panel => !report.panels?.includes(panel))) {
    throw new Error(`AppImage did not launch successfully: ${result.error || result.stderr || result.stdout}`);
  }
  console.log(JSON.stringify({ status: 'ok', actualAppImageRuntime: true, sandboxDisabled: false, panels: report.panels, backendVersion: report.backend.backendVersion }));
} finally { rmSync(root, { recursive: true, force: true }); }
