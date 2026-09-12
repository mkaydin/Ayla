// ── Code Analysis Panel ────────────────────────────────────
// Local Tree-sitter analysis, durable run history, and explicit graph/vector commit.

import { createPanel } from './panel-system.js';
import { escapeHtml } from '../lib/dom.js';
import { renderMarkdown } from '../lib/markdown.js';
import {
  hasDesktopBackend,
  invoke,
  invokeCancelable,
  listenEvent,
  selectFiles,
} from '../lib/ipc.js';
import { events } from '../lib/events.js';

const state = {
  sourceType: 'directory',
  directoryPath: '',
  code: '',
  language: 'rust',
  snippetName: 'snippet.rs',
  scope: 'all',
  semanticMode: 'deterministic',
  status: 'idle',
  progress: 0,
  stage: '',
  error: '',
  operation: null,
  processId: '',
  result: null,
  previewKind: 'human',
  preview: '',
  previewTruncated: false,
  approving: false,
  approval: null,
  removing: false,
  removal: null,
  runs: [],
  historyLoading: false,
  historyError: '',
  reopening: '',
  deleting: '',
};

let panelRef = null;

export function initCodeAnalysisPanel() {
  panelRef = createPanel('code', 'Code Analyzer', {
    x: Math.round(window.innerWidth * 0.17),
    y: Math.round(window.innerHeight * 0.1),
    width: 760,
    height: 620,
    minWidth: 460,
    minHeight: 380,
    className: 'code-analysis-panel',
  });
  render();
  loadRunHistory();
  return panelRef;
}

function render() {
  if (!panelRef) return;
  const body = panelRef.getContentEl();
  const running = state.status === 'running' || state.status === 'cancelling';
  body.innerHTML = `
    <div class="code-notice">
      Deterministic analysis runs inside Ayla. Ollama semantic mode may send selected excerpts to the configured cloud model. Remote Git is disabled; graph-v2 and code vectors change only after approval.
    </div>
    ${renderHistory()}
    <div class="code-source-tabs">
      <button class="code-tab ${state.sourceType === 'directory' ? 'active' : ''}" data-source="directory">Local directory</button>
      <button class="code-tab ${state.sourceType === 'snippet' ? 'active' : ''}" data-source="snippet">Code snippet</button>
    </div>
    ${state.sourceType === 'directory' ? directoryForm() : snippetForm()}
    <div class="code-options-row">
      <label>Analysis scope
        <select id="code-scope" ${running ? 'disabled' : ''}>
          <option value="all" ${state.scope === 'all' ? 'selected' : ''}>Code and documentation</option>
          <option value="documents" ${state.scope === 'documents' ? 'selected' : ''}>Documentation only</option>
        </select>
      </label>
      <label>Semantic enrichment
        <select id="code-semantic" ${running ? 'disabled' : ''}>
          <option value="deterministic" ${state.semanticMode === 'deterministic' ? 'selected' : ''}>Off (deterministic)</option>
          <option value="ollama" ${state.semanticMode === 'ollama' ? 'selected' : ''}>Ollama semantic (cloud enabled)</option>
        </select>
      </label>
      <span class="code-local-badge">No Git clone · semantic excerpts only</span>
    </div>
    <div class="code-actions">
      <button class="code-primary" id="code-run" ${running ? 'disabled' : ''}>Analyze and preview</button>
      ${running ? '<button class="code-danger" id="code-cancel">Cancel</button>' : ''}
    </div>
    ${renderStatus()}
    ${state.result ? renderResult() : ''}
  `;
  bind(body);
}

function renderHistory() {
  const runs = state.runs.slice(0, 12);
  return `
    <details class="code-history" ${runs.length ? '' : 'open'}>
      <summary>Run history <span>${state.historyLoading ? 'Loading…' : `${state.runs.length} saved`}</span></summary>
      ${state.historyError ? `<div class="code-history-error">${escapeHtml(state.historyError)}</div>` : ''}
      ${!state.historyLoading && !runs.length ? '<div class="code-history-empty">No saved code-analysis runs yet.</div>' : ''}
      <div class="code-history-list">
        ${runs.map(run => `
          <div class="code-history-item ${state.result?.pid === run.operationId ? 'active' : ''}">
            <button class="code-history-open" data-open-run="${escapeHtml(run.operationId)}" ${state.reopening ? 'disabled' : ''}>
              <strong>${escapeHtml(run.projectName || run.sourceLabel || 'Code analysis')}</strong>
              <span>${escapeHtml(formatRunDate(run.createdAt))} · ${Number(run.entityCount || 0).toLocaleString()} entities · ${escapeHtml(run.semanticMode || 'deterministic')}</span>
              <span>${escapeHtml(run.status || 'preview')}${run.parentOperationId ? ' · re-analysis' : ''}</span>
            </button>
            <button class="code-history-delete" title="Delete this run and all related data" data-delete-run="${escapeHtml(run.operationId)}" ${state.deleting ? 'disabled' : ''}>Delete</button>
          </div>
        `).join('')}
      </div>
    </details>
  `;
}

function formatRunDate(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? String(value || '') : date.toLocaleString();
}

function directoryForm() {
  return `
    <div class="code-directory-row">
      <input id="code-directory" type="text" readonly value="${escapeHtml(state.directoryPath)}" placeholder="Choose a project directory" />
      <button id="code-browse" ${['running', 'cancelling'].includes(state.status) ? 'disabled' : ''}>Browse…</button>
    </div>
  `;
}

function snippetForm() {
  const disabled = ['running', 'cancelling'].includes(state.status) ? 'disabled' : '';
  return `
    <div class="code-snippet-meta">
      <label>Name <input id="code-name" type="text" maxlength="200" value="${escapeHtml(state.snippetName)}" ${disabled} /></label>
      <label>Language
        <select id="code-language" ${disabled}>
          ${['rust', 'typescript', 'javascript', 'python'].map(language => (
            `<option value="${language}" ${state.language === language ? 'selected' : ''}>${language}</option>`
          )).join('')}
        </select>
      </label>
    </div>
    <textarea id="code-snippet" maxlength="2097152" spellcheck="false" placeholder="Paste source code here…" ${disabled}>${escapeHtml(state.code)}</textarea>
  `;
}

function renderStatus() {
  if (state.status === 'idle' && !state.error) return '';
  if (state.error) return `<div class="code-status error">${escapeHtml(state.error)}</div>`;
  if (state.status === 'complete') return '<div class="code-status success">Preview generated. Review it before graph import.</div>';
  const label = state.status === 'cancelling' ? 'Cancelling…' : (state.stage || 'Starting analysis…');
  return `
    <div class="code-status">
      <div class="code-progress-label"><span>${escapeHtml(label)}</span><span>${state.progress}%</span></div>
      <div class="code-progress"><div style="width:${state.progress}%"></div></div>
    </div>
  `;
}

function renderResult() {
  const result = state.result;
  const diagnostics = Array.isArray(result.diagnostics) ? result.diagnostics : [];
  const receipt = state.approval?.receipt;
  const removedReceipt = state.removal?.receipt;
  const imported = Boolean(result.graphApproved || receipt);
  const diff = result.diff;
  const canReanalyze = state.sourceType === 'directory'
    || Boolean(state.code.trim())
    || Boolean(result.sourceSnapshotStored);
  return `
    <section class="code-result">
      <div class="code-summary">
        <strong>${escapeHtml(result.projectName || 'Code analysis')}</strong>
        <span>${Number(result.entityCount || 0).toLocaleString()} entities</span>
        <span>${Number(result.relationCount || 0).toLocaleString()} relations</span>
        <span>${escapeHtml(result.semanticMode || 'deterministic')}</span>
        ${result.sourceSnapshotStored ? '<span>private snippet snapshot saved</span>' : ''}
      </div>
      ${diff ? `<div class="code-diff-summary">Compared with ${escapeHtml(diff.parentOperationId)}: +${diff.addedEntities} / −${diff.removedEntities} / ~${diff.changedEntities} entities; +${diff.addedRelations} / −${diff.removedRelations} / ~${diff.changedRelations} relations.</div>` : ''}
      ${diagnostics.length ? `<details class="code-diagnostics"><summary>${diagnostics.length} diagnostics</summary>${diagnostics.map(item => `<div>${escapeHtml(item.level)}: ${escapeHtml(item.message)}</div>`).join('')}</details>` : ''}
      <div class="code-preview-tabs">
        <button class="code-tab ${state.previewKind === 'human' ? 'active' : ''}" data-preview="human">Documentation</button>
        <button class="code-tab ${state.previewKind === 'knowledgeBase' ? 'active' : ''}" data-preview="knowledgeBase">Knowledge base</button>
        <button class="code-tab ${state.previewKind === 'structured' ? 'active' : ''}" data-preview="structured">Structured JSON</button>
        ${result.artifacts?.diff ? `<button class="code-tab ${state.previewKind === 'diff' ? 'active' : ''}" data-preview="diff">Diff</button>` : ''}
      </div>
      ${state.previewTruncated ? '<div class="code-truncated">Preview limited to 2 MiB; the complete artifact remains stored locally.</div>' : ''}
      <div class="code-preview ${['structured', 'diff'].includes(state.previewKind) ? 'structured' : ''}">
        ${['structured', 'diff'].includes(state.previewKind)
          ? `<pre>${escapeHtml(state.preview || 'Loading…')}</pre>`
          : renderMarkdown(state.preview || 'Loading…')}
      </div>
      <div class="code-result-actions">
        <button id="code-reanalyze" ${!canReanalyze || ['running', 'cancelling'].includes(state.status) ? 'disabled' : ''}>Re-analyze and diff</button>
        ${!canReanalyze ? '<span>This older run has no private source snapshot; paste the original snippet to re-analyze it.</span>' : ''}
      </div>
      <div class="code-approval">
        <div>
          <strong>Graph-v2 import</strong>
          <span>${receipt
            ? `Committed ${receipt.node_count ?? 0} nodes and ${receipt.edge_count ?? 0} edges.`
            : result.graphApproved
              ? `Previously committed${result.vectorsIndexed ? ' with entity/document vectors' : ''}.`
            : removedReceipt
              ? `Run provenance removed; graph-v2 now has ${removedReceipt.node_count ?? 0} nodes.`
              : 'This does not modify the legacy knowledge graph.'}</span>
        </div>
        ${imported
          ? `<button class="code-danger" id="code-remove" ${state.removing ? 'disabled' : ''}>${state.removing ? 'Removing…' : 'Remove import'}</button>`
          : `<button class="code-approve" id="code-approve" ${state.approving ? 'disabled' : ''}>${state.approving ? 'Importing…' : removedReceipt ? 'Import again' : 'Approve import'}</button>`}
      </div>
    </section>
  `;
}

function bind(body) {
  body.querySelectorAll('[data-source]').forEach(button => {
    button.addEventListener('click', () => {
      if (['running', 'cancelling'].includes(state.status)) return;
      captureInputs(body);
      state.sourceType = button.dataset.source;
      state.error = '';
      render();
    });
  });
  body.querySelector('#code-browse')?.addEventListener('click', chooseDirectory);
  body.querySelector('#code-run')?.addEventListener('click', () => runAnalysis(body));
  body.querySelector('#code-reanalyze')?.addEventListener('click', () => runAnalysis(body, state.result?.pid));
  body.querySelector('#code-cancel')?.addEventListener('click', cancelAnalysis);
  body.querySelector('#code-approve')?.addEventListener('click', approveImport);
  body.querySelector('#code-remove')?.addEventListener('click', removeImport);
  body.querySelectorAll('[data-open-run]').forEach(button => {
    button.addEventListener('click', () => reopenRun(button.dataset.openRun));
  });
  body.querySelectorAll('[data-delete-run]').forEach(button => {
    button.addEventListener('click', () => deleteRun(button.dataset.deleteRun));
  });
  body.querySelectorAll('[data-preview]').forEach(button => {
    button.addEventListener('click', () => loadPreview(button.dataset.preview));
  });
}

function captureInputs(body) {
  state.directoryPath = body.querySelector('#code-directory')?.value ?? state.directoryPath;
  state.code = body.querySelector('#code-snippet')?.value ?? state.code;
  state.language = body.querySelector('#code-language')?.value ?? state.language;
  state.snippetName = body.querySelector('#code-name')?.value ?? state.snippetName;
  state.scope = body.querySelector('#code-scope')?.value ?? state.scope;
  state.semanticMode = body.querySelector('#code-semantic')?.value ?? state.semanticMode;
}

async function chooseDirectory() {
  try {
    const selected = await selectFiles({ directory: true, multiple: false });
    if (Array.isArray(selected) && typeof selected[0] === 'string') {
      state.directoryPath = selected[0];
      state.error = '';
      render();
    }
  } catch (error) {
    state.error = error.message || String(error);
    render();
  }
}

async function runAnalysis(body, parentOperationId = null) {
  captureInputs(body);
  if (!hasDesktopBackend()) {
    state.error = 'Code analysis requires the Rust desktop backend.';
    render();
    return;
  }
  if (state.sourceType === 'directory' && !state.directoryPath) {
    state.error = 'Choose a local project directory first.';
    render();
    return;
  }
  if (state.sourceType === 'snippet'
      && !state.code.trim()
      && !(parentOperationId && state.result?.sourceSnapshotStored)) {
    state.error = 'Paste a code snippet first.';
    render();
    return;
  }

  state.status = 'running';
  state.progress = 1;
  state.stage = 'Preparing analysis';
  state.error = '';
  state.result = null;
  state.approval = null;
  state.removal = null;
  state.preview = '';
  state.processId = '';
  render();

  let unlistenStarted = () => {};
  let unlistenProgress = () => {};
  try {
    unlistenStarted = await listenEvent('code-analysis-started', event => {
      const payload = event.payload || {};
      if (payload.requestId && payload.requestId !== state.operation?.requestId) return;
      if (payload.pid) {
        state.processId = payload.pid;
        state.operation?.setProcessId(payload.pid);
      }
    });
    unlistenProgress = await listenEvent('code-analysis-progress', event => {
      const update = event.payload || {};
      if (state.processId && update.pid && update.pid !== state.processId) return;
      state.stage = update.message || update.stage || 'Analyzing';
      const current = Number(update.current || 0);
      const total = Number(update.total || 0);
      if (update.stage === 'preparing') state.progress = 3;
      else if (update.stage === 'rendering') state.progress = 95;
      else if (update.stage === 'complete') state.progress = 100;
      else if (total > 0) state.progress = Math.min(92, 8 + Math.round((current / total) * 84));
      render();
    });

    state.operation = invokeCancelable('run_code_analysis', {
      sourceType: state.sourceType,
      directoryPath: state.sourceType === 'directory' ? state.directoryPath : null,
      code: state.sourceType === 'snippet' && state.code.trim() ? state.code : null,
      language: state.sourceType === 'snippet' ? state.language : null,
      name: state.sourceType === 'snippet' ? state.snippetName : null,
      scope: state.scope,
      semanticMode: state.semanticMode,
      parentOperationId,
    });
    if (state.processId) state.operation.setProcessId(state.processId);
    const result = await state.operation.promise;
    state.result = result;
    state.processId = result.pid || state.processId;
    state.status = 'complete';
    state.progress = 100;
    state.previewKind = 'human';
    await loadPreview('human', false);
    await loadRunHistory(false);
    events.emit('status-log', {
      type: 'success',
      message: `Code analysis complete: ${result.entityCount || 0} entities, ${result.relationCount || 0} relations`,
    });
  } catch (error) {
    const wasCancelling = state.status === 'cancelling';
    state.status = 'idle';
    state.error = wasCancelling || error?.code === 'request_cancelled' || error?.code === 'code_analysis_cancelled'
      ? 'Code analysis cancelled.'
      : (error.message || String(error));
  } finally {
    try { unlistenStarted(); } catch {}
    try { unlistenProgress(); } catch {}
    state.operation = null;
    render();
  }
}

async function cancelAnalysis() {
  if (!state.operation) return;
  state.status = 'cancelling';
  state.stage = 'Cancelling analysis';
  render();
  await state.operation.cancel().catch(() => false);
}

function artifactFor(kind) {
  if (kind === 'knowledgeBase') return state.result?.artifacts?.knowledge_base;
  if (kind === 'diff') return state.result?.artifacts?.diff;
  return state.result?.artifacts?.[kind];
}

async function loadPreview(kind, shouldRender = true) {
  state.previewKind = kind;
  state.preview = '';
  state.previewTruncated = false;
  if (shouldRender) render();
  const artifact = artifactFor(kind);
  if (!artifact?.id) {
    state.error = `Missing ${kind} artifact.`;
    render();
    return;
  }
  try {
    const content = await invoke('get_artifact_content', { artifactId: artifact.id });
    state.preview = content.content || '';
    state.previewTruncated = Boolean(content.truncated);
  } catch (error) {
    state.error = error.message || String(error);
  }
  render();
}

async function approveImport() {
  if (!state.result?.pid || state.approving || state.approval || state.result.graphApproved) return;
  state.approving = true;
  state.error = '';
  render();
  try {
    state.removal = null;
    state.approval = await invoke('approve_code_analysis', {
      operationId: state.result.pid,
    });
    state.result.graphApproved = true;
    state.result.vectorsIndexed = true;
    await loadRunHistory(false);
    events.emit('kb-refresh');
    events.emit('status-log', {
      type: 'success',
      message: `Code analysis imported into graph-v2 and indexed (${state.approval.entityVectors || 0} entity vectors, ${state.approval.documentVectors || 0} document vectors). View it in Knowledge Base → Structured or Graph v2.`,
    });
  } catch (error) {
    state.error = error.message || String(error);
  } finally {
    state.approving = false;
    render();
  }
}

async function removeImport() {
  if (!state.result?.pid || state.removing || (!state.approval && !state.result.graphApproved)) return;
  state.removing = true;
  state.error = '';
  render();
  try {
    state.removal = await invoke('remove_code_analysis', {
      operationId: state.result.pid,
    });
    state.approval = null;
    state.result.graphApproved = false;
    await loadRunHistory(false);
    events.emit('kb-refresh');
    events.emit('status-log', {
      type: 'success',
      message: 'Code-analysis provenance removed from graph-v2',
    });
  } catch (error) {
    state.error = error.message || String(error);
  } finally {
    state.removing = false;
    render();
  }
}

async function loadRunHistory(shouldRender = true) {
  if (!hasDesktopBackend()) return;
  state.historyLoading = true;
  state.historyError = '';
  if (shouldRender) render();
  try {
    const runs = await invoke('get_code_analysis_runs');
    state.runs = Array.isArray(runs) ? runs : [];
  } catch (error) {
    state.historyError = error.message || String(error);
  } finally {
    state.historyLoading = false;
    render();
  }
}

async function reopenRun(operationId) {
  if (!operationId || state.reopening || ['running', 'cancelling'].includes(state.status)) return;
  state.reopening = operationId;
  state.error = '';
  render();
  try {
    const result = await invoke('get_code_analysis_run', { operationId });
    const record = state.runs.find(run => run.operationId === operationId);
    state.result = result;
    state.sourceType = record?.sourceType || 'directory';
    state.directoryPath = record?.directoryPath || '';
    state.language = record?.language || state.language;
    state.snippetName = record?.sourceLabel || state.snippetName;
    state.scope = record?.scope || state.scope;
    const restoredSemanticMode = record?.semanticMode || result.semanticMode || 'deterministic';
    state.semanticMode = restoredSemanticMode === 'local' ? 'ollama' : restoredSemanticMode;
    if (state.sourceType === 'snippet') state.code = '';
    state.status = 'complete';
    state.approval = null;
    state.removal = null;
    state.previewKind = 'human';
    await loadPreview('human', false);
  } catch (error) {
    state.error = error.message || String(error);
  } finally {
    state.reopening = '';
    render();
  }
}

async function deleteRun(operationId) {
  if (!operationId || state.deleting || ['running', 'cancelling'].includes(state.status)) return;
  const record = state.runs.find(run => run.operationId === operationId);
  const label = record?.projectName || record?.sourceLabel || operationId;
  if (!window.confirm(`Delete “${label}” and its graph import, vectors, artifacts, and history? This cannot be undone.`)) return;
  state.deleting = operationId;
  state.error = '';
  render();
  try {
    await invoke('delete_code_analysis_run', { operationId });
    if (state.result?.pid === operationId) {
      state.result = null;
      state.preview = '';
      state.approval = null;
      state.removal = null;
      state.status = 'idle';
    }
    await loadRunHistory(false);
    events.emit('kb-refresh');
    events.emit('status-log', {
      type: 'success',
      message: `Deleted code-analysis run ${label}`,
    });
  } catch (error) {
    state.error = error.message || String(error);
  } finally {
    state.deleting = '';
    render();
  }
}
