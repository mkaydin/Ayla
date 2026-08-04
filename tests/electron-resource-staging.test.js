import assert from 'node:assert/strict';
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  resourceManifest,
  stageResources,
} from '../electron/stage-resources.mjs';

function fixture() {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'alice-stage-'));
  const source = path.join(directory, 'source');
  const stageDirectory = path.join(directory, 'stage');
  mkdirSync(source);
  const files = {
    ALICE_SIDECAR_BIN: path.join(source, 'sidecar'),
    ALICE_OCR_BIN: path.join(source, 'ocr'),
    ALICE_SEARCH_BIN: path.join(source, 'search'),
  };
  for (const filePath of Object.values(files)) {
    writeFileSync(filePath, 'fixture');
    chmodSync(filePath, 0o644);
  }
  return { directory, stageDirectory, files };
}

test('Electron resource staging uses overrides and preserves executable resources', () => {
  const { directory, stageDirectory, files } = fixture();
  const manifest = stageResources({
    root: directory,
    stageDirectory,
    env: files,
  });
  assert.equal(manifest.length, 3);
  for (const item of manifest) {
    assert.equal(statSync(item.destination).isFile(), true);
    if (process.platform !== 'win32') {
      assert.notEqual(statSync(item.destination).mode & 0o111, 0);
    }
  }
});

test('Electron resource staging reports every missing dependency and build hint', () => {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'alice-stage-missing-'));
  const manifest = resourceManifest({
    root: directory,
    stageDirectory: path.join(directory, 'stage'),
    env: {},
  });
  assert.equal(manifest.length, 3);
  assert.throws(
    () => stageResources({
      root: directory,
      stageDirectory: path.join(directory, 'stage'),
      env: {},
    }),
    error => (
      error.message.includes('Rust sidecar')
      && error.message.includes('OCR helper')
      && error.message.includes('Meta Search helper')
      && error.message.includes('cargo build --release')
    ),
  );
});
