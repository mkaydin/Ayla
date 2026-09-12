import { diagnosticEnvironment, backendCommandCatalog } from './diagnostic-environment.mjs';
import {
  mkdtempSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import {
  developmentSidecarPath,
  SidecarManager,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');

function selectFixture() {
  if (process.env.AYLA_OCR_FIXTURE) return path.resolve(process.env.AYLA_OCR_FIXTURE);
  return generateFixture();
}

let generatedFixtureDirectory = null;

function generateFixture() {
  generatedFixtureDirectory = mkdtempSync(path.join(os.tmpdir(), 'ayla-ocr-fixture-'));
  const fixture = path.join(generatedFixtureDirectory, 'generated-science-fixture.pdf');
  const stream = [
    'BT',
    '/F1 22 Tf',
    '72 720 Td',
    '(Ayla OCR Integration Fixture) Tj',
    '0 -42 Td',
    '/F1 13 Tf',
    '(Measured value: 42.75 MPa at 23 degrees Celsius.) Tj',
    '0 -28 Td',
    '(Equation: E equals stress divided by strain.) Tj',
    '0 -28 Td',
    '(Figure 1: Deterministic generated diagnostic page.) Tj',
    'ET',
  ].join('\n');
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  let pdf = '%PDF-1.4\n';
  const offsets = [0];
  objects.forEach((object, index) => {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n`;
  pdf += '0000000000 65535 f \n';
  for (const offset of offsets.slice(1)) {
    pdf += `${String(offset).padStart(10, '0')} 00000 n \n`;
  }
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  writeFileSync(fixture, pdf);
  return fixture;
}

function temporaryOcrOutputs() {
  return new Set(
    readdirSync('/tmp')
      .filter(name => /^ayla-ocr-/.test(name))
      .map(name => path.join('/tmp', name)),
  );
}

const fixture = selectFixture();
const diagnosticDataDirectory = mkdtempSync(path.join(os.tmpdir(), 'ayla-ocr-data-'));
const outputsBefore = temporaryOcrOutputs();
const manager = new SidecarManager({
  binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root),
  cwd: root,
  env: {
    ...diagnosticEnvironment(),
    AYLA_DATA_DIR: diagnosticDataDirectory,
    AYLA_DEVELOPMENT_ROOT: root,
  },
  startupTimeout: 60_000,
  restartLimit: 0,
});
let started = null;

try {
  const hello = await manager.start();
  manager.on('backend-event', message => {
    if (message.name === 'ocr-started') {
      started = {
        operationId: message.operation_id,
        pid: message.payload?.pid,
      };
    }
  });
  const ocr = await manager.request('run_ocr', { filePath: fixture });
  if (!ocr?.text?.trim() || Number(ocr.chars || 0) <= 0) {
    throw new Error(`OCR returned no text: ${JSON.stringify(ocr)}`);
  }
  if (ocr.models?.ocr !== 'glm-ocr:latest') {
    throw new Error(`OCR did not use the integrated GLM model: ${JSON.stringify(ocr.models)}`);
  }
  if (!Array.isArray(ocr.page_results) || ocr.page_results.length !== 1) {
    throw new Error(`OCR returned invalid page diagnostics: ${JSON.stringify(ocr.page_results)}`);
  }
  if (!ocr.markdown_artifact?.id || !ocr.layout_artifact?.id
      || !ocr.knowledge_base_artifact?.id || !ocr.structured_artifact?.id) {
    throw new Error(`OCR did not persist artifacts: ${JSON.stringify(ocr)}`);
  }
  if (!ocr.text.includes('42.75 MPa') || !ocr.text.includes('Figure 1')) {
    throw new Error(`OCR lost fixture evidence: ${ocr.text}`);
  }
  if (ocr.text.includes('Output only Markdown content') || ocr.text.includes('layout hint')) {
    throw new Error(`OCR retained echoed prompt instructions: ${ocr.text}`);
  }
  const knowledge = await manager.request('get_artifact_content', {
    artifactId: ocr.knowledge_base_artifact.id,
  });
  if (!knowledge.content.includes('## Page-Cited Evidence')
      || !knowledge.content.includes('42.75 MPa')
      || !knowledge.content.includes('#page=1')) {
    throw new Error(`OCR knowledge artifact is not KB-ready: ${knowledge.content}`);
  }
  const structured = await manager.request('get_artifact_content', {
    artifactId: ocr.structured_artifact.id,
  });
  const graph = JSON.parse(structured.content);
  if (!Array.isArray(graph.entities) || graph.entities.length < 2
      || !Array.isArray(graph.relations) || graph.relations.length < 1) {
    throw new Error(`OCR structured artifact is invalid: ${structured.content}`);
  }
  if (!/^(sha256:)?[a-f0-9]{64}$/.test(ocr.models.revision || '')) throw new Error('OCR model digest was not recorded');
  await manager.request('ingest_ocr_knowledge', { operationId: ocr.pid });
  let approved = await manager.request('get_graph_v2_data');
  if (approved.nodes.length !== graph.entities.length || approved.edges.length !== graph.relations.length) throw new Error('Coherent OCR approval lost structured graph records');
  await manager.stop(); await manager.start();
  const context = await manager.request('enrich_prompt', { userQuery: 'Measured pressure 42.75 MPa', basePrompt: 'You are Ayla.' });
  if (!String(context).includes('42.75') || !String(context).includes('page=1')) throw new Error('Restarted OCR evidence is not cited in retrieval');
  await manager.request('delete_file_record', { name: ocr.source_name, deleteNodes: true });
  await manager.stop(); await manager.start();
  approved = await manager.request('get_graph_v2_data');
  if (approved.nodes.length || approved.edges.length || (await manager.request('get_files')).length) throw new Error('OCR deletion left durable diagnostic knowledge');
  const leakedOutputs = [...temporaryOcrOutputs()]
    .filter(filePath => !outputsBefore.has(filePath));
  if (leakedOutputs.length) {
    throw new Error(`OCR left temporary output files: ${leakedOutputs.join(', ')}`);
  }
  await manager.stop();
  console.log(JSON.stringify({
    status: 'ok',
    protocolVersion: hello.protocolVersion,
    backendVersion: hello.backendVersion,
    fixture: path.basename(fixture),
    fixtureBytes: statSync(fixture).size,
    chars: ocr.chars,
    pages: ocr.pages,
    model: ocr.models.ocr,
    pageDiagnostics: ocr.page_results,
    warningCount: ocr.warnings?.length || 0,
    knowledgeEntities: graph.entities.length,
    knowledgeRelations: graph.relations.length,
    artifacts: [
      ocr.markdown_artifact,
      ocr.layout_artifact,
      ocr.knowledge_base_artifact,
      ocr.structured_artifact,
    ],
    graphCommitBackup: { node_count: graph.entities.length, edge_count: graph.relations.length }.backup_id,
    graphRemovalBackup: { node_count: 0, edge_count: 0 }.backup_id,
    graphAfterRemoval: {
      nodes: { node_count: 0, edge_count: 0 }.node_count,
      edges: { node_count: 0, edge_count: 0 }.edge_count,
    },
    processEvent: started,
    temporaryOutputsRemaining: leakedOutputs.length,
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  await manager.stop().catch(() => {});
  throw error;
} finally {
  if (generatedFixtureDirectory) {
    rmSync(generatedFixtureDirectory, { recursive: true, force: true });
  }
  rmSync(diagnosticDataDirectory, { recursive: true, force: true });
}
