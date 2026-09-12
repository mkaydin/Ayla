// ── Document Processor Panel ───────────────────────────────
// OCR tab: GLM OCR with PDFium layout hints for scanned and scientific PDFs.
// Quick Extract tab: direct text read for .txt .md .json .docx .pdf (fast)

import { createPanel, getPanel } from './panel-system.js';
import { renderMarkdown } from '../lib/markdown.js';
import {
  invoke,
  invokeCancelable,
  hasDesktopBackend,
  listenEvent,
  selectFiles,
} from '../lib/ipc.js';
import { notifyProcess } from '../lib/process-notify.js';
import { events } from '../lib/events.js';
import { escapeHtml } from '../lib/dom.js';

let queueItems = [];
let activeMethod = 'quick_extract';
let ocrKnowledgeMode = 'semantic';

const FILE_ICONS = { '.pdf': '\uD83D\uDCD5', '.txt': '\uD83D\uDCC4', '.md': '\uD83D\uDCDD', '.json': '\uD83D\uDCCB', '.docx': '\uD83D\uDCD8' };

export function resetOcrView() {
  const panel = getPanel('ocr');
  if (panel) renderList(panel);
}

export function initOcrPanel() {
  const panel = createPanel('ocr', 'Document Processor', {
    x: Math.round(window.innerWidth * 0.05),
    y: Math.round(window.innerHeight * 0.15),
    width: 540, height: 520,
    minWidth: 400, minHeight: 300,
    className: 'ocr-panel',
    onClose: () => {},
  });

  renderList(panel);
  return panel;
}

function renderList(panel) {
  const body = panel.getContentEl();
  body.innerHTML = `
    <div class="ocr-method-tabs">
      <button class="ocr-method-tab ${activeMethod === 'quick_extract' ? 'active' : ''}" data-method="quick_extract">
        \u26A1 Quick Extract
      </button>
      <button class="ocr-method-tab ${activeMethod === 'ocr' ? 'active' : ''}" data-method="ocr">
        \uD83D\uDD6E OCR (Science PDFs)
      </button>
    </div>
    ${activeMethod === 'ocr' ? `<label class="ocr-kb-mode">
      <span>Knowledge output</span>
      <select id="ocr-kb-mode-select">
        <option value="deterministic" ${ocrKnowledgeMode === 'deterministic' ? 'selected' : ''}>Structured (deterministic)</option>
        <option value="semantic" ${ocrKnowledgeMode === 'semantic' ? 'selected' : ''}>Structured + Ollama semantic assembly</option>
        <option value="disabled" ${ocrKnowledgeMode === 'disabled' ? 'selected' : ''}>Evidence document only</option>
      </select>
      <small>Semantic mode uses the configured Ollama model. Cloud models receive bounded OCR page evidence; deterministic output is preserved if enrichment fails.</small>
    </label>` : ''}
    <div class="ocr-upload-zone" id="ocr-upload-zone">
      <div class="ocr-upload-icon">${activeMethod === 'ocr' ? '\uD83D\uDCD5' : '\uD83D\uDCC2'}</div>
      <div class="ocr-upload-text">${activeMethod === 'ocr' ? 'Drop PDF here or click to browse' : 'Drop .txt .md .json .docx .pdf or click'}</div>
      <div class="ocr-upload-hint">${activeMethod === 'ocr' ? 'Max 100 MB per file — local GLM OCR with PDFium layout detection' : 'Direct text extraction — Qdrant vector storage'}</div>
    </div>
    <div class="ocr-queue-section-title">History <span style="font-weight:400;color:var(--text-muted);font-size:10px">(${queueItems.length})</span></div>
    <div id="ocr-queue"></div>
  `;

  body.querySelectorAll('.ocr-method-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      activeMethod = tab.dataset.method;
      renderList(panel);
    });
  });
  body.querySelector('#ocr-kb-mode-select')?.addEventListener('change', (event) => {
    ocrKnowledgeMode = event.target.value;
  });

  setupUpload(body, panel);
  renderQueue();
}

function setupUpload(body, panel) {
  const zone = document.getElementById('ocr-upload-zone');
  if (!zone) return;

  const extensions = activeMethod === 'ocr' ? ['pdf'] : ['pdf', 'txt', 'md', 'json', 'docx'];

  zone.addEventListener('click', async () => {
    try {
      const selected = await selectFiles({
        multiple: true,
        filters: [{ name: 'Documents', extensions }],
      });
      if (selected === null) {
        openBrowserPicker();
        return;
      }
      for (const path of selected) {
        if (typeof path === 'string') {
          queueFile({ name: path.split(/[/\\]/).pop() || path, path, size: 0 });
        }
      }
    } catch (e) {
      console.warn('Native document picker failed:', e);
      openBrowserPicker();
    }
  });

  function openBrowserPicker() {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = extensions.map(e => '.' + e).join(',');
    input.multiple = true;
    input.addEventListener('change', () => {
      for (const file of input.files) {
        queueFile({ name: file.name, path: file.path || null, file, size: file.size });
      }
    });
    input.click();
  }

  zone.addEventListener('dragover', (e) => { e.preventDefault(); zone.classList.add('dragover'); });
  zone.addEventListener('dragleave', () => zone.classList.remove('dragover'));
  zone.addEventListener('drop', async (e) => {
    e.preventDefault();
    zone.classList.remove('dragover');
    for (const item of e.dataTransfer.items) {
      if (item.kind === 'file') {
        const file = item.getAsFile();
        if (file) queueFile({ name: file.name, path: file.path || null, file, size: file.size });
      }
    }
    if (e.dataTransfer.files.length > 0) {
      for (const file of e.dataTransfer.files) {
        const already = queueItems.find(i => i.filename === file.name && i.timestamp > Date.now() - 1000);
        if (!already) queueFile({ name: file.name, path: file.path || null, file, size: file.size });
      }
    }
  });
}

function queueFile(fileInfo) {
  const ext = (fileInfo.name || '').split('.').pop()?.toLowerCase();
  const validOcr = activeMethod === 'ocr' && ext === 'pdf';
  const validQuick = activeMethod === 'quick_extract' && ['pdf', 'txt', 'md', 'json', 'docx', 'rs', 'js', 'py', 'toml', 'yaml', 'yml', 'html', 'css', 'xml', 'csv', 'log'].includes(ext);

  if (!validOcr && !validQuick) {
    notifyProcess('error', 'doc-process', { filename: fileInfo.name, reason: 'Format not supported in ' + activeMethod + ' mode' });
    return;
  }
  if (fileInfo.size > 100 * 1024 * 1024) {
    notifyProcess('error', 'doc-process', { filename: fileInfo.name, reason: 'File exceeds 100 MB limit' });
    return;
  }

  const item = {
    id: crypto.randomUUID(),
    filename: fileInfo.name,
    path: fileInfo.path || fileInfo.name,
    file: fileInfo.file || null,
    size: fileInfo.size || 0,
    method: activeMethod,
    knowledgeMode: activeMethod === 'ocr' ? ocrKnowledgeMode : null,
    status: 'queued',
    pages: '?',
    chars: 0,
    chunks: 0,
    progress: 0,
    text: '',
    stage: '',
    warnings: [],
    pageResults: [],
    model: '',
    assemblyModel: '',
    performance: null,
    markdownArtifact: null,
    layoutArtifact: null,
    knowledgeBaseArtifact: null,
    structuredArtifact: null,
    knowledgeEntityCount: 0,
    knowledgeRelationCount: 0,
    knowledgePreview: null,
    structuredPreview: null,
    previewMode: 'faithful',
    graphApproval: null,
    graphRemoval: null,
    graphBusy: false,
    error: null,
    timestamp: Date.now(),
  };
  queueItems.push(item);
  renderQueue();
  processNextInQueue();
}

async function processNextInQueue() {
  const processing = queueItems.filter(i => i.status === 'processing').length;
  if (processing > 0) return;

  const item = queueItems.find(i => i.status === 'queued');
  if (!item) return;

  item.status = 'processing';
  item.cancelRequested = false;
  renderQueue();

  const label = item.method === 'ocr' ? 'OCR' : 'Quick Extract';
  notifyProcess('start', 'doc-process', {
    filename: item.filename,
    method: label,
    operationId: item.id,
  });
  let unlistenStarted = () => {};
  let unlistenProgress = () => {};

  try {
    if (!hasDesktopBackend()) {
      throw new Error('Document processing requires the Rust desktop backend');
    }
    if (!item.path) {
      throw new Error('A native local file path is required for document processing');
    }
    events.emit('status-log', { type: 'info', message: label + ': processing ' + item.filename + '...' });

    if (item.method === 'ocr') {
      unlistenStarted = await listenEvent('ocr-started', (event) => {
        if (
          event.payload?.requestId
          && event.payload.requestId !== item.operation?.requestId
        ) return;
        if (event.payload?.pid) {
          item.processId = event.payload.pid;
          item.operation?.setProcessId(item.processId);
        }
      });
      unlistenProgress = await listenEvent('ocr-progress', (event) => {
        const update = event.payload || {};
        if (item.processId && update.pid && update.pid !== item.processId) return;
        const current = Number(update.current_page || 0);
        const total = Number(update.total_pages || 0);
        item.stage = update.message || update.stage || 'Processing';
        if (update.stage === 'rendering') item.progress = 5;
        else if (update.stage === 'cache' && total > 0) {
          item.progress = Math.min(15, 5 + Math.round((current / total) * 10));
        }
        else if (update.stage === 'recognizing' && total > 0) {
          item.progress = Math.min(90, 10 + Math.round((current / total) * 80));
        } else if (update.stage === 'assembling') item.progress = 95;
        else if (update.stage === 'semantic_assembly') item.progress = 97;
        else if (update.stage === 'complete') item.progress = 100;
        renderQueue();
      });
      item.operation = invokeCancelable('run_ocr', {
        filePath: item.path,
        knowledgeMode: item.knowledgeMode,
      });
      if (item.cancelRequested) await item.operation.cancel().catch(() => false);
      const result = await item.operation.promise;
      item.text = result.text || '';
      item.chars = item.text.length;
      item.pages = result.pages || '?';
      item.processId = result.pid || null;
      item.warnings = Array.isArray(result.warnings) ? result.warnings : [];
      item.pageResults = Array.isArray(result.page_results) ? result.page_results : [];
      item.model = result.models?.ocr || '';
      item.assemblyModel = result.models?.assembly || '';
      item.knowledgeMode = result.knowledge_mode || item.knowledgeMode;
      item.performance = result.performance || null;
      item.markdownArtifact = result.markdown_artifact || null;
      item.layoutArtifact = result.layout_artifact || null;
      item.knowledgeBaseArtifact = result.knowledge_base_artifact || null;
      item.structuredArtifact = result.structured_artifact || null;
      item.knowledgeEntityCount = Number(result.knowledge_entity_count || 0);
      item.knowledgeRelationCount = Number(result.knowledge_relation_count || 0);
      item.progress = 100;
    } else {
      item.operation = invokeCancelable('run_quick_extract', { filePath: item.path });
      if (item.cancelRequested) await item.operation.cancel().catch(() => false);
      const result = await item.operation.promise;
      item.text = result.text || '';
      item.chars = result.chars || item.text.length;
      item.chunks = result.chunks || 0;
    }

    item.status = 'complete';
    events.emit('status-log', { type: 'success', message: label + ': ' + item.filename + ' done — ' + item.chars.toLocaleString() + ' chars' + (item.chunks ? ', ' + item.chunks + ' chunks' : '') });
    notifyProcess('complete', 'doc-process', {
      filename: item.filename,
      method: label,
      chars: item.chars,
      chunks: item.chunks,
      operationId: item.id,
    });
  } catch (e) {
    item.status = 'failed';
    item.error = item.cancelRequested || e?.code === 'request_cancelled'
      ? 'Cancelled by user'
      : e.message || String(e);
    notifyProcess(item.cancelRequested || e?.code === 'request_cancelled' ? 'cancel' : 'error', 'doc-process', {
      filename: item.filename,
      method: label,
      reason: item.error,
      operationId: item.id,
    });
  } finally {
    try { unlistenStarted(); } catch {}
    try { unlistenProgress(); } catch {}
    delete item.operation;
    item.cancelRequested = false;
  }

  renderQueue();
  processNextInQueue();
}

function renderQueue() {
  const container = document.getElementById('ocr-queue');
  if (!container) return;
  container.innerHTML = '';

  if (queueItems.length === 0) {
    container.innerHTML = '<div style="color:var(--text-muted);font-size:var(--font-size-xs);padding:var(--space-md);text-align:center">No documents processed yet.</div>';
    return;
  }

  for (const item of queueItems) {
    const el = document.createElement('div');
    const icon = FILE_ICONS['.' + (item.filename.split('.').pop() || '')] || '\uD83D\uDCC4';
    const methodBadge = item.method === 'ocr' ? '\uD83D\uDD6E OCR' : '\u26A1 Quick';
    const ocrStats = item.method === 'ocr'
      ? ` \u00B7 ${escapeHtml(item.knowledgeMode || 'deterministic')}${item.performance?.pages_from_cache ? ` \u00B7 ${item.performance.pages_from_cache} cached` : ''}`
      : '';
    const modelStats = item.model
      ? ` \u00B7 ${escapeHtml(item.model)}${item.assemblyModel ? ` + ${escapeHtml(item.assemblyModel)}` : ''}`
      : '';
    el.className = 'ocr-queue-item ' + (item.status === 'complete' ? 'complete' : item.status === 'failed' ? 'failed' : '');
    el.innerHTML = `
      <div class="ocr-item-header">
        <span class="ocr-item-name">${icon} ${escapeHtml(item.filename)}</span>
        <span class="ocr-item-status ${item.status}">${item.cancelRequested ? 'Cancelling\u2026' : statusLabel(item.status)}</span>
      </div>
      <div class="ocr-item-meta">
        <span class="ocr-method-badge">${methodBadge}</span>
        ${item.pages !== '?' ? item.pages + ' pages \u00B7 ' : ''}${item.chars.toLocaleString()} chars${item.chunks ? ' \u00B7 ' + item.chunks + ' chunks' : ''}${ocrStats}${item.knowledgeEntityCount ? ' \u00B7 KB ' + item.knowledgeEntityCount + ' entities/' + item.knowledgeRelationCount + ' relations' : ''}${modelStats}${item.warnings?.length ? ' \u00B7 \u26A0 ' + item.warnings.length : ''} \u00B7 ${timeAgo(item.timestamp)}
      </div>
      ${item.status === 'processing' ? `
        <div class="ocr-item-progress"><div class="ocr-item-progress-fill" style="width:${item.progress}%"></div></div>
        ${item.stage ? `<div class="ocr-item-meta">${escapeHtml(item.stage)}</div>` : ''}
        <div class="ocr-item-actions">
          <button class="ocr-action-btn discard" data-action="cancel" data-id="${item.id}"
            ${item.cancelRequested ? 'disabled' : ''}>
            ${item.cancelRequested ? '\u23F3 Cancelling\u2026' : '\u23F9 Stop'}
          </button>
        </div>` : ''}
      ${item.status === 'complete' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="view" data-id="${item.id}">\uD83D\uDC41 View</button>
          <button class="ocr-action-btn approve" data-action="approve" data-id="${item.id}">\u2713 Approve</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">\u2717 Hide</button>
        </div>` : ''}
      ${item.status === 'ingested' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="view" data-id="${item.id}">\uD83D\uDCC4 View Text</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">\u2717 Hide</button>
        </div>` : ''}
      ${item.status === 'failed' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="retry" data-id="${item.id}">\u21BA Retry</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">\u2717 Hide</button>
        </div>
        <div style="font-size:10px;color:#fca5a5;margin-top:4px">${escapeHtml(item.error || 'Unknown error')}</div>` : ''}
    `;

    el.querySelectorAll('.ocr-action-btn').forEach(btn => {
      btn.addEventListener('click', () => {
        const action = btn.dataset.action;
        const id = btn.dataset.id;
        const target = queueItems.find(i => i.id === id);
        if (!target) return;

        if (action === 'view') renderPreview(target);
        else if (action === 'approve') approveItem(target);
        else if (action === 'discard') discardItem(target);
        else if (action === 'retry') { target.status = 'queued'; renderQueue(); processNextInQueue(); }
        else if (action === 'cancel') {
          events.emit('status-log', { type: 'info', message: 'Cancelling ' + target.filename });
          target.cancelRequested = true;
          void target.operation?.cancel().catch(() => false);
          renderQueue();
        }
      });
    });

    container.appendChild(el);
  }
}

function renderPreview(item, mode = item.previewMode || 'faithful') {
  const panel = getPanel('ocr');
  if (!panel) return;
  item.previewMode = mode;
  const body = panel.getContentEl();
  const isOcr = item.method === 'ocr';
  const preview = mode === 'knowledge'
    ? item.knowledgePreview
    : mode === 'structured'
      ? item.structuredPreview
      : item.text;
  body.innerHTML = `
    <button class="ocr-back-btn" id="ocr-back-btn">\u2190 Back to list</button>
    ${isOcr ? `<div class="ocr-method-tabs ocr-preview-tabs">
      <button class="ocr-method-tab ${mode === 'faithful' ? 'active' : ''}" data-preview="faithful">Faithful OCR</button>
      <button class="ocr-method-tab ${mode === 'knowledge' ? 'active' : ''}" data-preview="knowledge">KB-ready document</button>
      <button class="ocr-method-tab ${mode === 'structured' ? 'active' : ''}" data-preview="structured">Structured graph</button>
    </div>` : ''}
    <div class="ocr-preview" id="ocr-preview">${preview == null
      ? '<div class="ocr-preview-loading">Loading artifact…</div>'
      : mode === 'structured'
        ? `<pre>${escapeHtml(preview)}</pre>`
        : renderMarkdown(preview)}</div>
    ${item.error ? `<div class="ocr-preview-error">${escapeHtml(item.error)}</div>` : ''}
    ${item.warnings?.length ? `<details class="ocr-diagnostics"><summary>${item.warnings.length} OCR warning${item.warnings.length === 1 ? '' : 's'}</summary><ul>${item.warnings.map(warning => `<li>${warning.page ? `Page ${warning.page}: ` : ''}${escapeHtml(warning.message || warning.code)}</li>`).join('')}</ul></details>` : ''}
    ${isOcr ? `<div class="ocr-kb-summary">
      <strong>${item.knowledgeMode === 'disabled' ? 'OCR evidence document' : 'Structured OCR knowledge'}</strong>
      <span>${item.knowledgeEntityCount} entities · ${item.knowledgeRelationCount} relations · page-cited${item.performance?.pages_from_cache ? ` · ${item.performance.pages_from_cache} pages restored from cache` : ''}</span>
    </div>
    <div class="ocr-graph-approval"><span>${item.status === 'ingested' ? 'Evidence, graph, and vectors approved in the knowledge base.' : 'Approve the page-cited evidence, structured graph, and search vectors together.'}</span></div>` : ''}
    <div class="ocr-preview-footer">
      ${item.status !== 'ingested' ? `<button class="ocr-action-btn approve" id="ocr-approve-btn">\u2713 ${isOcr ? 'Approve KB-ready document' : 'Approve &amp; Add to Knowledge Base'}</button>` : '<span class="ocr-kb-ingested">Knowledge document added to Ayla.</span>'}
      <button class="ocr-action-btn discard" id="ocr-discard-btn">\u2717 Hide from history</button>
    </div>
  `;

  document.getElementById('ocr-back-btn').addEventListener('click', () => renderList(panel));
  document.getElementById('ocr-approve-btn')?.addEventListener('click', () => approveItem(item));
  document.getElementById('ocr-discard-btn').addEventListener('click', () => discardItem(item));
  body.querySelectorAll('[data-preview]').forEach(button => {
    button.addEventListener('click', () => renderPreview(item, button.dataset.preview));
  });

  if (isOcr && mode === 'knowledge' && item.knowledgePreview == null) {
    void loadOcrArtifact(item, 'knowledge');
  } else if (isOcr && mode === 'structured' && item.structuredPreview == null) {
    void loadOcrArtifact(item, 'structured');
  }
}

async function loadOcrArtifact(item, mode) {
  const artifact = mode === 'knowledge' ? item.knowledgeBaseArtifact : item.structuredArtifact;
  if (!artifact?.id) {
    item.error = `Missing OCR ${mode} artifact`;
    renderPreview(item, mode);
    return;
  }
  try {
    const result = await invoke('get_artifact_content', { artifactId: artifact.id });
    if (mode === 'knowledge') item.knowledgePreview = result.content || '';
    else item.structuredPreview = result.content || '';
    item.previewTruncated = Boolean(result.truncated);
  } catch (error) {
    if (mode === 'knowledge') item.knowledgePreview = `Could not load artifact: ${error.message || error}`;
    else item.structuredPreview = JSON.stringify({ error: error.message || String(error) }, null, 2);
  }
  if (item.previewMode === mode) renderPreview(item, mode);
}

async function approveItem(item) {
  if (item.status === 'processing' || item.status === 'ingested') return;
  const label = item.method === 'ocr' ? 'ocr' : 'quick_extract';
  const announcementData = {
    kind: 'document',
    source: item.filename,
    target: 'ocr',
    operationId: item.id,
  };
  notifyProcess('start', 'kb-ingest', announcementData);
  item.status = 'processing';
  item.progress = 0;
  item.cancelRequested = false;
  renderQueue();

  if (!hasDesktopBackend()) {
    item.status = 'failed';
    item.error = 'Knowledge ingestion requires the Rust desktop backend';
    notifyProcess('error', 'kb-ingest', { ...announcementData, reason: item.error });
    renderQueue();
    return;
  }

  try {
      const unlisten = await listenEvent('ingest-progress', (event) => {
        item.progress = Math.max(item.progress || 20, event.payload.pct ?? 20);
        const step = event.payload.step || 'processing';
        events.emit('status-log', { type: 'info', message: 'Extracting ' + step + '...' });
        renderQueue();
      });

      try {
        item.progress = 10; renderQueue();
        if (item.method === 'ocr') {
          events.emit('status-log', { type: 'info', message: 'Ingesting the KB-ready OCR artifact...' });
          item.operation = invokeCancelable('ingest_ocr_knowledge', {
            operationId: item.processId,
          });
          item.operation.setProcessId(item.processId);
        } else {
          events.emit('status-log', { type: 'info', message: 'Saving file record...' });
          item.operation = invokeCancelable('commit_document', {
            name: item.filename, path: item.path, chars: item.chars,
            sourceType: label, text: item.text,
          });
        }
        if (item.cancelRequested) await item.operation.cancel().catch(() => false);
        const result = await item.operation.promise;

        item.progress = 100; renderQueue();
        notifyProcess('complete', 'kb-ingest', announcementData);
        events.emit('status-log', { type: 'success', message: 'KB: ' + (result.message || result) });
        events.emit('kb-refresh');
      } finally {
        delete item.operation;
        item.cancelRequested = false;
        // Always release the ingest-progress listener, even on error (issue H13 — previously
        // each failed approve re-registered a ghost handler that mutated stale `item` refs).
        try { unlisten(); } catch {}
      }
  } catch (e) {
    console.warn('Ingest failed:', e);
    item.status = 'failed';
    item.error = e.message || String(e);
    notifyProcess('error', 'kb-ingest', { ...announcementData, reason: e.message || String(e) });
    renderQueue();
    return;
  }

  item.status = 'ingested';
  renderQueue();
  const panel = getPanel('ocr');
  if (panel) renderList(panel);

  const kbPanel = getPanel('kb');
  if (kbPanel) {
    events.emit('kb-refresh');
  }
}

function discardItem(item) {
  queueItems = queueItems.filter(i => i.id !== item.id);
  renderQueue();
  const panel = getPanel('ocr');
  if (panel && panel.getContentEl().querySelector('#ocr-preview')) renderList(panel);
}

function statusLabel(status) {
  const map = { queued: 'Queued', processing: 'Processing...', complete: 'Complete', failed: 'Failed', ingested: 'In KB' };
  return map[status] || status;
}

function timeAgo(ts) {
  const s = Math.floor((Date.now() - ts) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return Math.floor(s / 60) + 'm ago';
  if (s < 86400) return Math.floor(s / 3600) + 'h ago';
  return Math.floor(s / 86400) + 'd ago';
}
