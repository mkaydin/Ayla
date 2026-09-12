import { diagnosticEnvironment, backendCommandCatalog } from './diagnostic-environment.mjs';
import path from 'node:path';
import process from 'node:process';
import { tmpdir } from 'node:os';
import {
  developmentSidecarPath,
  SidecarManager,
  SidecarRequestError,
} from './sidecar-manager.mjs';

const root = path.resolve(import.meta.dirname, '..');
const manager = new SidecarManager({
  binaryPath: process.env.AYLA_SIDECAR_BIN || developmentSidecarPath(root),
  cwd: root,
  env: {
    ...diagnosticEnvironment(),
    AYLA_DATA_DIR: path.join(tmpdir(), 'ayla-code-analysis-live-diagnostic'),
    AYLA_DEVELOPMENT_ROOT: root,
    // Never write diagnostics into the user's normal Ayla graph namespace.
  },
  startupTimeout: 10_000,
});

try {
  await manager.start();
  const result = await manager.request('run_code_analysis', {
    sourceType: 'snippet',
    directoryPath: null,
    code: 'struct DiagnosticNode;\nimpl DiagnosticNode { fn verify(&self) {} }\n',
    language: 'rust',
    name: 'graph-v2-live-diagnostic.rs',
    scope: 'all',
  });
  const approval = await manager.request('approve_code_analysis', {
    operationId: result.pid,
  });
  if (approval?.receipt?.node_count < result.entityCount) {
    throw new Error(`Graph-v2 commit lost entities: ${JSON.stringify(approval)}`);
  }
  if (approval.entityVectors < result.entityCount || approval.documentVectors < 2) {
    throw new Error(`Dedicated code-analysis vectors are incomplete: ${JSON.stringify(approval)}`);
  }
  const importedGraph = await manager.request('get_graph_v2_data');
  const importedNodes = importedGraph.nodes.filter(node =>
    node.sources?.includes(approval.sourceId));
  if (!importedGraph.available || importedNodes.length !== result.entityCount
      || !importedGraph.runs.some(run =>
        run.action === 'import' && run.source_id === approval.sourceId)) {
    throw new Error(`Graph-v2 application read lost the import: ${JSON.stringify(importedGraph)}`);
  }
  const nodePage = await manager.request('get_graph_v2_page', {
    kind: 'nodes',
    offset: 0,
    limit: 1,
  });
  if (nodePage.nodes?.length !== 1 || !nodePage.hasMore || nodePage.nextOffset !== 1) {
    throw new Error(`Graph-v2 node pagination is invalid: ${JSON.stringify(nodePage)}`);
  }
  const enriched = await manager.request('enrich_prompt', { userQuery: 'Explain DiagnosticNode', basePrompt: 'You are Ayla.' });
  if (!String(enriched).includes('DiagnosticNode') || !String(enriched).includes('source:')) {
    throw new Error('Approved code knowledge was not retrieved with source provenance');
  }
  const removal = await manager.request('remove_code_analysis', {
    operationId: result.pid,
  });
  if (!removal?.receipt?.backup_id) {
    throw new Error(`Graph-v2 removal did not return a backup: ${JSON.stringify(removal)}`);
  }
  const removedGraph = await manager.request('get_graph_v2_data');
  if (removedGraph.nodes.some(node => node.sources?.includes(approval.sourceId))
      || !removedGraph.runs.some(run =>
        run.action === 'remove' && run.source_id === approval.sourceId)) {
    throw new Error(`Graph-v2 application read lost the removal: ${JSON.stringify(removedGraph)}`);
  }
  const rollback = await manager.request('rollback_graph_v2', {
    backupId: removal.receipt.backup_id,
  });
  const restoredGraph = await manager.request('get_graph_v2_data');
  if (!restoredGraph.nodes.some(node => node.sources?.includes(approval.sourceId))
      || !restoredGraph.runs.some(run =>
        run.action === 'rollback'
        && run.target_backup_id === removal.receipt.backup_id)) {
    throw new Error(`Graph-v2 rollback was not visible: ${JSON.stringify(restoredGraph)}`);
  }
  const finalRemoval = await manager.request('remove_code_analysis', {
    operationId: result.pid,
  });
  if (finalRemoval.receipt?.node_count !== 0 || finalRemoval.receipt?.edge_count !== 0) {
    throw new Error(`Graph-v2 rollback cleanup left records: ${JSON.stringify(finalRemoval)}`);
  }
  const migration = await manager.request('migrate_legacy_graph_v2');
  if (migration?.sourceId !== 'legacy-mirror:v1'
      || !migration.receipt?.backup_id) {
    throw new Error(`Legacy Graph-v2 migration is invalid: ${JSON.stringify(migration)}`);
  }
  const runPage = await manager.request('get_graph_v2_page', {
    kind: 'runs',
    offset: 0,
    limit: 2,
  });
  if (runPage.runs?.length !== 2 || !runPage.hasMore || runPage.nextOffset !== 2) {
    throw new Error(`Graph-v2 run pagination is invalid: ${JSON.stringify(runPage)}`);
  }
  const pruning = await manager.request('prune_graph_v2_backups', { retention: 3 });
  if (pruning.retained > 3 || pruning.pruned < 1) {
    throw new Error(`Graph-v2 backup retention was not enforced: ${JSON.stringify(pruning)}`);
  }
  const maintainedGraph = await manager.request('get_graph_v2_data');
  if (!maintainedGraph.runs.some(run => run.backup_available === false)) {
    throw new Error(`Pruned backup state is missing from audit history: ${JSON.stringify(maintainedGraph.runs)}`);
  }
  const expiredRun = maintainedGraph.runs.find(run => run.backup_available === false);
  try {
    await manager.request('rollback_graph_v2', { backupId: expiredRun.backup_id });
    throw new Error('Expired Graph-v2 backup unexpectedly restored');
  } catch (error) {
    if (!(error instanceof SidecarRequestError)
        || error.code !== 'graph_v2_rollback'
        || !error.message.includes('expired')) throw error;
  }
  const deletion = await manager.request('delete_code_analysis_run', {
    operationId: result.pid,
  });
  if (!deletion.artifactsRemoved || !deletion.vectorsRemoved) {
    throw new Error(`Run-level deletion was incomplete: ${JSON.stringify(deletion)}`);
  }
  const analyzerRuns = await manager.request('get_code_analysis_runs');
  if (analyzerRuns.some(run => run.operationId === result.pid)) {
    throw new Error(`Deleted analyzer run remains in history: ${JSON.stringify(analyzerRuns)}`);
  }

  const revisionBase = await manager.request('run_code_analysis', {
    sourceType: 'snippet',
    directoryPath: null,
    code: 'struct RevisionNode;\nimpl RevisionNode { fn first(&self) {} }\n',
    language: 'rust',
    name: 'revision-live-diagnostic.rs',
    scope: 'all',
  });
  const revisionBaseApproval = await manager.request('approve_code_analysis', {
    operationId: revisionBase.pid,
  });
  const revision = await manager.request('run_code_analysis', {
    sourceType: 'snippet',
    directoryPath: null,
    code: 'struct RevisionNode;\nimpl RevisionNode { fn first(&self) {} fn second(&self) {} }\n',
    language: 'rust',
    name: 'revision-live-diagnostic.rs',
    scope: 'all',
    parentOperationId: revisionBase.pid,
  });
  const revisionApproval = await manager.request('approve_code_analysis', {
    operationId: revision.pid,
  });
  const replacedGraph = await manager.request('get_graph_v2_data');
  if (replacedGraph.nodes.some(node => node.sources?.includes(revisionBaseApproval.sourceId))
      || replacedGraph.nodes.filter(node => node.sources?.includes(revisionApproval.sourceId)).length !== revision.entityCount
      || !replacedGraph.runs.some(run =>
        run.action === 'replace' && run.source_id === revisionApproval.sourceId)) {
    throw new Error(`Approved re-analysis did not replace its parent: ${JSON.stringify(replacedGraph)}`);
  }
  const revisionRuns = await manager.request('get_code_analysis_runs');
  const superseded = revisionRuns.find(run => run.operationId === revisionBase.pid);
  if (superseded?.status !== 'superseded' || superseded.graphApproved) {
    throw new Error(`Parent run was not marked superseded: ${JSON.stringify(superseded)}`);
  }
  await manager.request('delete_code_analysis_run', { operationId: revision.pid });
  await manager.request('delete_code_analysis_run', { operationId: revisionBase.pid });

  await manager.stop();
  console.log(JSON.stringify({
    status: 'ok',
    namespace: 'ayla_diagnostics',
    analyzedEntities: result.entityCount,
    analyzedRelations: result.relationCount,
    committedNodes: approval.receipt.node_count,
    committedEdges: approval.receipt.edge_count,
    commitBackup: approval.receipt.backup_id,
    remainingNodesAfterSourceRemoval: removal.receipt.node_count,
    remainingEdgesAfterSourceRemoval: removal.receipt.edge_count,
    removalBackup: removal.receipt.backup_id,
    rollbackRecoveryBackup: rollback.receipt.backup_id,
    visibleRuns: restoredGraph.runs.length,
    paginatedNodeCount: nodePage.nodes.length,
    paginatedRunCount: runPage.runs.length,
    legacyMigration: `${migration.migratedNodes}/${migration.migratedEdges}`,
    retainedBackups: pruning.retained,
    prunedBackups: pruning.pruned,
    entityVectors: approval.entityVectors,
    documentVectors: approval.documentVectors,
    runDeletion: deletion,
    revisionReplacement: {
      previousEntities: revisionBase.entityCount,
      currentEntities: revision.entityCount,
      diff: revision.diff,
    },
    cleanShutdown: manager.snapshot().state === 'stopped',
  }, null, 2));
} catch (error) {
  await manager.stop().catch(() => {});
  throw error;
}
