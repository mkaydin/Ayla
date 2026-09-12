import { mkdtempSync, existsSync, readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
const root = path.resolve(import.meta.dirname, '..');
const directory = mkdtempSync(path.join(tmpdir(), 'ayla-source-install-'));
const prefix = path.join(directory, 'path with spaces', 'ayla');
const integration = path.join(directory, 'user-integration');
const configRoot = path.join(directory, 'config');
function installer(args, timeout = 900000) {
  const result = spawnSync('bash', ['install.sh', ...args, '--prefix', prefix, '--integration-root', integration], {
    cwd: root, env: { ...process.env, XDG_CONFIG_HOME: configRoot }, encoding: 'utf8', timeout, maxBuffer: 10 * 1024 * 1024,
  });
  if (result.status !== 0) throw new Error(`Source installer failed: ${result.stderr.slice(-4000) || result.error}`);
  return result.stdout;
}
installer(['--source-dir', root]);
const settings = path.join(configRoot, 'ayla', 'settings.env');
const original = readFileSync(settings, 'utf8');
const state = JSON.parse(installer(['--status']));
if (!existsSync(state.launcher) || !existsSync(path.join(prefix, 'current', 'ayla'))) throw new Error('Installed launcher/payload is missing');
installer(['--uninstall']);
if (existsSync(state.launcher) || existsSync(path.join(prefix, 'current'))) throw new Error('Uninstall left desktop integration/payload');
if (readFileSync(settings, 'utf8') !== original) throw new Error('Uninstall altered user configuration');
console.log(JSON.stringify({ status: 'ok', actualSourceBuild: true, userInstallation: true, pathsWithSpaces: true, statusMetadata: true, uninstall: true, configurationPreserved: true }, null, 2));
