import path from 'node:path';
import { readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { diagnosticEnvironment } from './diagnostic-environment.mjs';
import { SidecarManager, developmentSidecarPath } from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const corpus = path.resolve(process.env.AYLA_OCR_CORPUS_DIR || path.join(root, 'release/ocr-corpus'));
const manifest = JSON.parse(await readFile(path.join(corpus, 'manifest.json'), 'utf8'));
const manager = new SidecarManager({ binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root), cwd: root, env: { ...diagnosticEnvironment(), AYLA_DEVELOPMENT_ROOT: root, AYLA_OCR_KB_MODE: 'deterministic', AYLA_OCR_ASSEMBLY_ALLOW_CLOUD: 'false' }, startupTimeout: 60000 });
const cases = [];
const reportPath = path.join(corpus, 'review-results.json');
await writeFile(reportPath, JSON.stringify({ timestamp: new Date().toISOString(), status: 'running', humanAcceptance: 'pending', cases }, null, 2) + '\n');
try {
  await manager.start();
  for (const fixture of manifest.cases) {
    if (!/^[a-z0-9-]+\.pdf$/.test(fixture.pdf) || !/^[a-z0-9-]+$/.test(fixture.name)) throw new Error('Invalid public fixture name');
    console.error(`Reviewing public fixture: ${fixture.name}`);
    const filePath = path.join(corpus, fixture.pdf);
    const sourceSha256 = createHash('sha256').update(await readFile(filePath)).digest('hex');
    let result;
    try {
      result = await manager.request('run_ocr', { filePath });
    } catch (error) {
      cases.push({ name: fixture.name, sourceSha256, error: { code: error.code, message: error.message }, manualReview: fixture.review, humanReviewed: false });
      await writeFile(path.join(corpus, `${fixture.name}.ocr.md`), `# OCR failed: ${fixture.name}\n\n${error.message}\n`);
      continue;
    }
    const text = String(result.text || '');
    await writeFile(path.join(corpus, `${fixture.name}.ocr.md`), text);
    const normalized = text.replace(/,/g, '.');
    const missingCriticalTokens = fixture.criticalTokens.filter(token => !normalized.includes(token.replace(/,/g, '.')));
    const missingExpectedWords = (fixture.expectedWords || []).filter(word => !text.includes(word));
    const occurrences = new Map();
    for (const line of text.split('\n').map(line => line.trim()).filter(line => line.length >= 12)) occurrences.set(line, (occurrences.get(line) || 0) + 1);
    const potentiallyRepeatedLines = [...occurrences].filter(([, count]) => count > 1).map(([line, count]) => ({ line, count }));
    cases.push({ name: fixture.name, sourceSha256, chars: result.chars, pages: result.pages, missingCriticalTokens, missingExpectedWords, potentiallyRepeatedLines, warnings: result.warnings, pageDiagnostics: result.pageResults, manualReview: fixture.review, humanReviewed: false });
  }
  const failed = cases.some(c => c.error || c.missingCriticalTokens?.length);
  const report = { timestamp: new Date().toISOString(), status: cases.some(c => c.error) ? 'ocr_failed' : failed ? 'regression_signal' : 'automated_tokens_passed', humanAcceptance: 'pending', cases, limits: manifest.limits };
  await writeFile(reportPath, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify(report, null, 2));
  if (failed) process.exitCode = 1;
} finally { await manager.stop(); }
