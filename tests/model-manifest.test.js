import assert from 'node:assert/strict';
import { existsSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '..');

test('bundled model manifest references complete in-repository resources', () => {
  const manifestPath = path.join(root, 'models', 'model-manifest.json');
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'));
  assert.equal(manifest.schemaVersion, 1);
  assert.ok(manifest.models.length > 0);

  for (const model of manifest.models) {
    assert.ok(model.id);
    assert.ok(model.license);
    assert.ok(model.source.startsWith('https://'));
    assert.equal(existsSync(path.join(root, model.licenseFile)), true);
    for (const file of model.files) {
      const filePath = path.join(root, file.path);
      assert.equal(existsSync(filePath), true, file.path);
      assert.equal(statSync(filePath).size, file.bytes, file.path);
      assert.match(file.sha256, /^[a-f0-9]{64}$/);
    }
  }
});

test('Supertonic model and code licenses remain distinct', () => {
  const directory = path.join(root, 'tts', 'supertonic3-int8');
  const modelLicense = readFileSync(path.join(directory, 'MODEL_LICENSE'), 'utf8');
  const repositoryLicense = readFileSync(
    path.join(directory, 'UPSTREAM_REPOSITORY_LICENSE'),
    'utf8',
  );
  assert.match(modelLicense, /Open RAIL-M License/);
  assert.match(repositoryLicense, /MIT License/);
});
