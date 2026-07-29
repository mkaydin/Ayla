// ── Document Processor Panel ───────────────────────────────
// OCR tab: deepseek-ocr for scanned PDFs (slow, thorough)
// Quick Extract tab: direct text read for .txt .md .json .docx .pdf (fast)

import { createPanel, getPanel } from './panel-system.js';
import { renderMarkdown } from '../lib/markdown.js';
import { invoke, getIpc } from '../lib/ipc.js';
import { notifyProcess } from '../lib/process-notify.js';
import { events } from '../lib/events.js';

let queueItems = [];
let activeMethod = 'quick_extract';

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
    <div class="ocr-upload-zone" id="ocr-upload-zone">
      <div class="ocr-upload-icon">${activeMethod === 'ocr' ? '\uD83D\uDCD5' : '\uD83D\uDCC2'}</div>
      <div class="ocr-upload-text">${activeMethod === 'ocr' ? 'Drop PDF here or click to browse' : 'Drop .txt .md .json .docx .pdf or click'}</div>
      <div class="ocr-upload-hint">${activeMethod === 'ocr' ? 'Max 100 MB per file — uses deepseek-ocr' : 'Direct text extraction — Qdrant vector storage'}</div>
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

  setupUpload(body, panel);
  renderQueue();
}

function setupUpload(body, panel) {
  const zone = document.getElementById('ocr-upload-zone');
  if (!zone) return;

  const extensions = activeMethod === 'ocr' ? ['pdf'] : ['pdf', 'txt', 'md', 'json', 'docx'];

  zone.addEventListener('click', async () => {
    if (window.__TAURI_INTERNALS__) {
      try {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({
          multiple: true,
          filters: [{ name: 'Documents', extensions }],
        });
        if (selected) {
          const files = Array.isArray(selected) ? selected : [selected];
          for (const path of files) {
            queueFile({ name: path.split('/').pop() || path, path, size: 0 });
          }
        }
      } catch (e) {
        openBrowserPicker();
      }
    } else {
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
    status: 'queued',
    pages: '?',
    chars: 0,
    chunks: 0,
    progress: 0,
    text: '',
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
  renderQueue();

  const label = item.method === 'ocr' ? 'OCR' : 'Quick Extract';
  notifyProcess('start', 'doc-process', { filename: item.filename, method: label });

  try {
    const ipcFn = getIpc();
    if (ipcFn && item.path) {
      events.emit('status-log', { type: 'info', message: label + ': processing ' + item.filename + '...' });

      if (item.method === 'ocr') {
        const result = await invoke('run_ocr', { filePath: item.path });
        item.text = result.text || '';
        item.chars = item.text.length;
        item.pages = result.pages || '?';
        item.processId = result.pid || null;
      } else {
        const result = await invoke('run_quick_extract', { filePath: item.path });
        item.text = result.text || '';
        item.chars = result.chars || item.text.length;
        item.chunks = result.chunks || 0;
      }

      item.status = 'complete';
      events.emit('status-log', { type: 'success', message: label + ': ' + item.filename + ' done — ' + item.chars.toLocaleString() + ' chars' + (item.chunks ? ', ' + item.chunks + ' chunks' : '') });
      notifyProcess('complete', 'doc-process', { filename: item.filename, chars: item.chars, chunks: item.chunks });
    } else {
      events.emit('status-log', { type: 'info', message: label + ': simulating for ' + item.filename + '...' });
      await new Promise(r => setTimeout(r, 800));
      item.text = '# ' + label + ' Output\n\nExtracted text for ' + item.filename + '.\n\nIn Tauri mode with Docker services up, the actual pipeline runs.';
      item.chars = item.text.length;
      item.chunks = item.method === 'quick_extract' ? 2 : 0;
      item.status = 'complete';
      events.emit('status-log', { type: 'success', message: label + ': ' + item.filename + ' simulated' });
      notifyProcess('complete', 'doc-process', { filename: item.filename, chars: item.chars });
    }
  } catch (e) {
    item.status = 'failed';
    item.error = e.message || String(e);
    notifyProcess('error', 'doc-process', { filename: item.filename, reason: item.error });
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
    el.className = 'ocr-queue-item ' + (item.status === 'complete' ? 'complete' : item.status === 'failed' ? 'failed' : '');
    el.innerHTML = `
      <div class="ocr-item-header">
        <span class="ocr-item-name">${icon} ${item.filename}</span>
        <span class="ocr-item-status ${item.status}">${statusLabel(item.status)}</span>
      </div>
      <div class="ocr-item-meta">
        <span class="ocr-method-badge">${methodBadge}</span>
        ${item.pages !== '?' ? item.pages + ' pages \u00B7 ' : ''}${item.chars.toLocaleString()} chars${item.chunks ? ' \u00B7 ' + item.chunks + ' chunks' : ''} \u00B7 ${timeAgo(item.timestamp)}
      </div>
      ${item.status === 'processing' ? `
        <div class="ocr-item-progress"><div class="ocr-item-progress-fill" style="width:${item.progress}%"></div></div>
        <div class="ocr-item-actions">
          <button class="ocr-action-btn discard" data-action="cancel" data-id="${item.id}">\u23F9 Stop</button>
        </div>` : ''}
      ${item.status === 'complete' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="view" data-id="${item.id}">\uD83D\uDC41 View</button>
          <button class="ocr-action-btn approve" data-action="approve" data-id="${item.id}">\u2713 Approve</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">\u2717 Discard</button>
        </div>` : ''}
      ${item.status === 'ingested' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="view" data-id="${item.id}">\uD83D\uDCC4 View Text</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">\u2717 Remove</button>
        </div>` : ''}
      ${item.status === 'failed' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="retry" data-id="${item.id}">\u21BA Retry</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">\u2717 Discard</button>
        </div>
        <div style="font-size:10px;color:#fca5a5;margin-top:4px">${item.error || 'Unknown error'}</div>` : ''}
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
          if (target.processId) invoke('cancel_process', { pid: target.processId }).catch(() => {});
          target.status = 'failed';
          target.error = 'Cancelled by user';
          renderQueue();
        }
      });
    });

    container.appendChild(el);
  }
}

function renderPreview(item) {
  const panel = getPanel('ocr');
  if (!panel) return;
  const body = panel.getContentEl();
  body.innerHTML = `
    <button class="ocr-back-btn" id="ocr-back-btn">\u2190 Back to list</button>
    <div class="ocr-preview" id="ocr-preview">${renderMarkdown(item.text)}</div>
    <div class="ocr-preview-footer">
      <button class="ocr-action-btn approve" id="ocr-approve-btn">\u2713 Approve &amp; Add to Knowledge Base</button>
      <button class="ocr-action-btn discard" id="ocr-discard-btn">\u2717 Discard</button>
    </div>
  `;

  document.getElementById('ocr-back-btn').addEventListener('click', () => renderList(panel));
  document.getElementById('ocr-approve-btn').addEventListener('click', () => approveItem(item));
  document.getElementById('ocr-discard-btn').addEventListener('click', () => discardItem(item));
}

async function approveItem(item) {
  const ipcFn = getIpc();
  const label = item.method === 'ocr' ? 'ocr' : 'quick_extract';
  notifyProcess('start', 'kb-ingest', { source: item.filename });
  item.status = 'processing';
  item.progress = 0;
  renderQueue();

  if (ipcFn) {
    try {
      item.progress = 10; renderQueue();
      events.emit('status-log', { type: 'info', message: 'Saving file record...' });
      await invoke('save_file_record', { name: item.filename, path: item.path, chars: item.chars, sourceType: label, text: item.text });

      const { listenEvent } = await import('../lib/ipc.js');
      const unlisten = await listenEvent('ingest-progress', (event) => {
        item.progress = Math.max(item.progress || 20, event.payload.pct ?? 20);
        const step = event.payload.step || 'processing';
        events.emit('status-log', { type: 'info', message: 'Extracting ' + step + '...' });
        renderQueue();
      });

      try {
        item.progress = 20; renderQueue();
        const result = await invoke('ingest_text', { text: item.text, source: item.filename, sourceType: label });

        item.progress = 100; renderQueue();
        notifyProcess('complete', 'kb-ingest', { source: item.filename });
        events.emit('status-log', { type: 'success', message: 'KB: ' + result });
      } finally {
        // Always release the ingest-progress listener, even on error (issue H13 — previously
        // each failed approve re-registered a ghost handler that mutated stale `item` refs).
        try { unlisten(); } catch {}
      }
    } catch (e) {
      console.warn('Ingest failed:', e);
      notifyProcess('error', 'kb-ingest', { reason: e.message || String(e) });
    }
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
