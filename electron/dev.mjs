import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import electronPath from 'electron';

const electronDirectory = dirname(fileURLToPath(import.meta.url));
const projectDirectory = dirname(electronDirectory);
const viteEntry = join(projectDirectory, 'node_modules', 'vite', 'bin', 'vite.js');
const mainEntry = join(electronDirectory, 'main.mjs');
const developmentUrl = 'http://127.0.0.1:5173';

if (!existsSync(viteEntry)) {
  throw new Error('Vite is not installed. Run `npm install` first.');
}

const vite = spawn(process.execPath, [
  viteEntry,
  '--host', '127.0.0.1',
  '--port', '5173',
  '--strictPort',
], {
  cwd: projectDirectory,
  env: process.env,
  stdio: 'inherit',
});

async function waitForVite() {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    if (vite.exitCode !== null) {
      throw new Error(`Vite exited before Electron started (code ${vite.exitCode})`);
    }
    try {
      const response = await fetch(developmentUrl);
      if (response.ok) return;
    } catch {
      // Vite is still starting.
    }
    await new Promise(resolve => setTimeout(resolve, 250));
  }
  throw new Error('Timed out waiting for Vite on port 5173');
}

let electron = null;
let stopping = false;

function stop(signal = 'SIGTERM') {
  if (stopping) return;
  stopping = true;
  if (electron && electron.exitCode === null) electron.kill(signal);
  if (vite.exitCode === null) vite.kill(signal);
}

process.on('SIGINT', () => stop('SIGINT'));
process.on('SIGTERM', () => stop('SIGTERM'));
vite.on('error', error => {
  console.error('Vite failed to start:', error);
  stop();
  process.exitCode = 1;
});

try {
  await waitForVite();
  electron = spawn(electronPath, [mainEntry], {
    cwd: projectDirectory,
    env: {
      ...process.env,
      AYLA_ELECTRON_DEV_URL: developmentUrl,
    },
    stdio: 'inherit',
  });
  electron.on('error', error => {
    console.error('Electron failed to start:', error);
    process.exitCode = 1;
    stop();
  });
  electron.on('exit', code => {
    process.exitCode = code ?? 0;
    stop();
  });
} catch (error) {
  console.error(error);
  process.exitCode = 1;
  stop();
}
