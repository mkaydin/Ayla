import { diagnosticEnvironment, backendCommandCatalog } from './diagnostic-environment.mjs';
import path from 'node:path';
import process from 'node:process';
import { tmpdir } from 'node:os';
import { existsSync, statSync } from 'node:fs';
import {
  developmentSidecarPath,
  SidecarManager,
  SidecarRequestError,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const diagnosticDataDir = path.join(tmpdir(), 'ayla-code-analysis-diagnostic');
const manager = new SidecarManager({
  binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root),
  args: ['--offline'],
  cwd: root,
  env: {
    ...diagnosticEnvironment(),
    AYLA_DATA_DIR: diagnosticDataDir,
    AYLA_DEVELOPMENT_ROOT: root,
    AYLA_LLM_MODEL: 'diagnostic:cloud',
    AYLA_CODE_ANALYSIS_MODEL: 'diagnostic:cloud',
    AYLA_CODE_ANALYSIS_ALLOW_CLOUD: 'false',
  },
  startupTimeout: 5_000,
});

const progressStages = [];
manager.on('backend-event', message => {
  if (message.name === 'code-analysis-progress') {
    progressStages.push(message.payload?.stage);
  }
});

try {
  await manager.start();
  const result = await manager.request('run_code_analysis', {
    sourceType: 'snippet',
    directoryPath: null,
    code: 'use std::fmt;\nstruct Parser;\nimpl Parser { fn parse(&self) {} }\n',
    language: 'rust',
    name: 'diagnostic.rs',
    scope: 'all',
  });
  if (!result?.pid || result.entityCount < 2 || result.relationCount < 1) {
    throw new Error(`Invalid analyzer result: ${JSON.stringify(result)}`);
  }
  if (!result.sourceSnapshotStored) {
    throw new Error('Snippet run did not record a private source snapshot');
  }
  const privateSourcePath = path.join(
    diagnosticDataDir,
    'artifacts',
    'code-analysis',
    result.pid,
    'source.snippet',
  );
  if ((statSync(privateSourcePath).mode & 0o777) !== 0o600) {
    throw new Error('Private snippet snapshot permissions are not 0600');
  }
  if (!progressStages.includes('parsing') || !progressStages.includes('complete')) {
    throw new Error(`Missing analyzer progress stages: ${JSON.stringify(progressStages)}`);
  }

  const human = await manager.request('get_artifact_content', {
    artifactId: result.artifacts?.human?.id,
  });
  if (!human?.content?.includes('Parser')) {
    throw new Error('Human documentation artifact does not contain the parsed symbol');
  }
  const structured = await manager.request('get_artifact_content', {
    artifactId: result.artifacts?.structured?.id,
  });
  const parsed = JSON.parse(structured.content);
  if (parsed.entities.some(entity => entity.source_excerpt != null)) {
    throw new Error('Structured artifact retained a raw source excerpt');
  }
  try {
    await manager.request('get_artifact_content', {
      artifactId: `code-analysis:${result.pid}:source`,
    });
    throw new Error('Private snippet snapshot was exposed through the artifact preview API');
  } catch (error) {
    if (!(error instanceof SidecarRequestError) || error.code !== 'invalid_artifact_id') throw error;
  }
  const runs = await manager.request('get_code_analysis_runs');
  if (!runs.some(run => run.operationId === result.pid)) {
    throw new Error(`Persisted analyzer run is missing: ${JSON.stringify(runs)}`);
  }
  const reopened = await manager.request('get_code_analysis_run', {
    operationId: result.pid,
  });
  if (reopened.projectName !== result.projectName || reopened.entityCount !== result.entityCount) {
    throw new Error(`Reopened analyzer run changed: ${JSON.stringify(reopened)}`);
  }
  const replay = await manager.request('run_code_analysis', {
    sourceType: 'snippet',
    directoryPath: null,
    code: null,
    language: 'rust',
    name: 'diagnostic.rs',
    scope: 'all',
    semanticMode: 'deterministic',
    parentOperationId: result.pid,
  });
  if (!replay.sourceSnapshotStored
      || replay.diff?.addedEntities !== 0
      || replay.diff?.removedEntities !== 0
      || replay.diff?.changedEntities !== 0
      || replay.diff?.addedRelations !== 0
      || replay.diff?.removedRelations !== 0
      || replay.diff?.changedRelations !== 0) {
    throw new Error(`Private snapshot replay changed the analysis: ${JSON.stringify(replay)}`);
  }
  const revision = await manager.request('run_code_analysis', {
    sourceType: 'snippet',
    directoryPath: null,
    code: 'use std::fmt;\nstruct Parser;\nimpl Parser { fn parse(&self) {} fn reset(&mut self) {} }\n',
    language: 'rust',
    name: 'diagnostic.rs',
    scope: 'all',
    semanticMode: 'deterministic',
    parentOperationId: result.pid,
  });
  if (revision.parentOperationId !== result.pid || !revision.diff || !revision.artifacts?.diff) {
    throw new Error(`Re-analysis diff is missing: ${JSON.stringify(revision)}`);
  }
  const diff = await manager.request('get_artifact_content', {
    artifactId: revision.artifacts.diff.id,
  });
  if (JSON.parse(diff.content).summary?.parentOperationId !== result.pid) {
    throw new Error(`Diff artifact has the wrong parent: ${diff.content}`);
  }

  let approvalContract = 'graph-v2-offline';
  try {
    await manager.request('approve_code_analysis', { operationId: result.pid });
    throw new Error('Offline graph-v2 approval unexpectedly succeeded');
  } catch (error) {
    if (!(error instanceof SidecarRequestError) || error.code !== 'graph_v2_offline') throw error;
  }

  try {
    await manager.request('run_code_analysis', {
      sourceType: 'snippet',
      code: 'struct LocalOnly;',
      language: 'rust',
      name: 'local-only.rs',
      semanticMode: 'local',
    });
    throw new Error('Cloud-tagged semantic model unexpectedly passed the local-only gate');
  } catch (error) {
    if (!(error instanceof SidecarRequestError)
        || error.code !== 'local_semantic_model_invalid') throw error;
  }
  try {
    await manager.request('run_code_analysis', { sourceType: 'git' });
    throw new Error('Remote Git unexpectedly passed the disabled integration gate');
  } catch (error) {
    if (!(error instanceof SidecarRequestError)
        || error.code !== 'remote_code_source_disabled') throw error;
  }

  await manager.request('delete_code_analysis_run', { operationId: replay.pid });
  await manager.request('delete_code_analysis_run', { operationId: revision.pid });
  await manager.request('delete_code_analysis_run', { operationId: result.pid });
  const remainingRuns = await manager.request('get_code_analysis_runs');
  if (remainingRuns.some(run =>
    run.operationId === result.pid
    || run.operationId === replay.pid
    || run.operationId === revision.pid)) {
    throw new Error(`Deleted analyzer history remains: ${JSON.stringify(remainingRuns)}`);
  }
  if (existsSync(privateSourcePath)) {
    throw new Error('Private snippet snapshot remained after full run deletion');
  }

  await manager.stop();
  console.log(JSON.stringify({
    status: 'ok',
    projectName: result.projectName,
    entities: result.entityCount,
    relations: result.relationCount,
    progressStages,
    humanArtifactBytes: result.artifacts.human.bytes,
    structuredArtifactBytes: result.artifacts.structured.bytes,
    rawSourcePubliclyExposed: false,
    privateSnippetSnapshot: '0600',
    snapshotReplay: true,
    approvalContract,
    localSemanticPrivacyGate: true,
    remoteGitDisabled: true,
    durableHistory: true,
    reanalysisDiff: revision.diff,
    runDeletion: true,
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  await manager.stop().catch(() => {});
  throw error;
}
