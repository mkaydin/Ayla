import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '..');
const frontendModules = [
  'src/main.js',
  'src/tts-player.js',
  'src/ui/chat-panel.js',
  'src/ui/kb-panel.js',
  'src/ui/kb/files-tab.js',
  'src/ui/kb/tables.js',
  'src/ui/ocr-panel.js',
  'src/ui/code-analysis-panel.js',
  'src/ui/search-panel.js',
  'src/ui/status-panel.js',
];

test('feature UI modules depend only on the runtime-neutral desktop adapter', () => {
  const forbidden = [
    ['direct Tauri package import', /@tauri-apps/],
    ['direct Tauri global access', /__TAURI/],
    ['direct Electron preload access', /window\.aylaDesktop/],
    ['raw IPC implementation access', /\bgetIpc\b/],
    ['direct Ollama proxy access', /\/api\/ollama/],
  ];

  for (const relativePath of frontendModules) {
    const source = readFileSync(path.join(root, relativePath), 'utf8');
    for (const [name, pattern] of forbidden) {
      assert.doesNotMatch(source, pattern, `${relativePath} contains ${name}`);
    }
  }
});

test('OCR, research, and code panels contain no simulated success path', () => {
  for (const relativePath of ['src/ui/ocr-panel.js', 'src/ui/search-panel.js', 'src/ui/code-analysis-panel.js']) {
    const source = readFileSync(path.join(root, relativePath), 'utf8');
    assert.doesNotMatch(source, /\bsimulat(?:e|ed|ing|ion)\b/i);
    assert.doesNotMatch(source, /browser preview/i);
  }
});
