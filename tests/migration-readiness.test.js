import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

const run = promisify(execFile);
const script = path.resolve(import.meta.dirname, '../scripts/migration-readiness.mjs');

async function fixture(t, settings = '') {
  const directory = await mkdtemp(path.join(tmpdir(), 'ayla-migration-review-test-'));
  const queries = [];
  const server = createServer(async (request, response) => {
    let body = '';
    for await (const chunk of request) body += chunk;
    queries.push(body);
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify([
      ...[12, 20, 3, 1, 0, 0, 2, 1, 4].map(total => ({ status: 'OK', result: [{ total }] })),
      { status: 'OK', result: [{ model: 'fixture-embedding', digest: 'public-fixture', dimensions: 1024 }] },
    ]));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const config = path.join(directory, 'settings.env');
  await writeFile(config, `SURREALDB_URL=http://127.0.0.1:${server.address().port}\nSURREALDB_NS=review_fixture\nSURREALDB_DB=review_fixture\n${settings}`);
  t.after(async () => {
    await new Promise(resolve => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  });
  const env = { PATH: process.env.PATH };
  return { config, queries, env };
}

test('migration review reports aggregates and sends only fixed read-only queries', async t => {
  const { config, queries, env } = await fixture(t);
  const { stdout } = await run(process.execPath, [script, '--config', config], { env, timeout: 5000 });
  const result = JSON.parse(stdout);
  assert.equal(result.readOnly, true);
  assert.equal(result.counts.graphNodes, 12);
  assert.equal(result.counts.failedApprovals, 1);
  assert.equal(result.reviewRequired.pendingOperations, true);
  assert.equal(result.reviewRequired.unscopedMemory, true);
  assert.equal(result.embeddingProfile.dimensions, 1024);
  assert.equal(queries.length, 1);
  assert.ok(queries[0].split(';').map(s => s.trim()).filter(Boolean).every(s => s.startsWith('SELECT ')));
  assert.doesNotMatch(queries[0], /SELECT \*|SELECT text|SELECT summary|SELECT messages/);
});

test('migration review rejects invalid storage identity before making requests', async t => {
  const { config, queries, env } = await fixture(t, 'SURREALDB_DB=invalid;REMOVE DATABASE other\n');
  await assert.rejects(run(process.execPath, [script, '--config', config], { env, timeout: 5000 }), /Invalid namespace\/database/);
  assert.equal(queries.length, 0);
});

test('migration review never falls back from an explicit missing settings file', async t => {
  const { config, queries, env } = await fixture(t);
  await assert.rejects(run(process.execPath, [script, '--config', `${config}.missing`], { env, timeout: 5000 }), /Explicit configuration file does not exist/);
  assert.equal(queries.length, 0);
});
