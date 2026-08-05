import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '..');
const read = relativePath => readFileSync(path.join(root, relativePath), 'utf8');

test('OCR and research are linked Rust libraries rather than helper processes', () => {
  const workspace = read('Cargo.toml');
  const coreManifest = read('crates/ayla-core/Cargo.toml');
  const coreApi = read('crates/ayla-core/src/api.rs');
  const research = read('crates/ayla-core/src/research.rs');
  const staging = read('electron/stage-resources.mjs');

  assert.match(workspace, /crates\/ayla-ocr/);
  assert.match(workspace, /crates\/ayla-research/);
  assert.match(coreManifest, /ayla-ocr\s*=\s*\{\s*path/);
  assert.match(coreManifest, /ayla-research\s*=\s*\{\s*path/);
  assert.match(coreApi, /ayla_ocr::extract_pdf/);
  assert.match(research, /ayla_research::search_with_progress/);
  assert.doesNotMatch(`${coreApi}\n${research}`, /Command::new|tokio::process/);
  assert.doesNotMatch(staging, /OCR helper|Meta Search helper|ocr-helper/);
});
