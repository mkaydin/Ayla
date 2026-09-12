import { spawnSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';

const root = path.resolve(import.meta.dirname, '..');
for (const name of ['surrealdb', 'qdrant', 'searxng']) await mkdir(path.join(root, 'data', name), { recursive: true, mode: 0o700 });
const environment = path.join(root, '.env');
if (!existsSync(environment)) {
  const template = await readFile(path.join(root, '.env.example'), 'utf8');
  await writeFile(environment, template.replace('change-me-to-a-long-random-string', randomBytes(32).toString('hex')).replace('change-me-to-a-64-hex-char-random-string', randomBytes(32).toString('hex')), { mode: 0o600 });
  console.log('Created private service configuration with generated credentials. Existing configuration is never overwritten.');
}
if (process.argv.includes('--start')) {
  const result = spawnSync('docker', ['compose', 'up', '-d'], { cwd: root, stdio: 'inherit', env: { ...process.env, AYLA_STORAGE_UID: String(process.getuid()), AYLA_STORAGE_GID: String(process.getgid()) } });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error('Local services could not be started; check Docker availability and user permissions');
} else console.log('Storage directories are ready. Run npm run services:setup -- --start to start the local service stack.');
