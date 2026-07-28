// ── OCR Panel ──────────────────────────────────────────────

import { createPanel, getPanel } from './panel-system.js';
import { renderMarkdown } from '../lib/markdown.js';
import { invoke, getIpc } from '../lib/ipc.js';
import { notifyProcess } from '../lib/process-notify.js';
import { events } from '../lib/events.js';

let queueItems = [];

export function resetOcrView() {
  const panel = getPanel('ocr');
  if (panel) renderList(panel);
}

export function initOcrPanel() {
  const panel = createPanel('ocr', 'OCR Document Processor', {
    x: Math.round(window.innerWidth * 0.05),
    y: Math.round(window.innerHeight * 0.15),
    width: 520, height: 500,
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
    <div class="ocr-upload-zone" id="ocr-upload-zone">
      <div class="ocr-upload-icon">\uD83D\uDCC4</div>
      <div class="ocr-upload-text">Drop PDF here or click to browse</div>
      <div class="ocr-upload-hint">Max 100 MB per file</div>
    </div>
    <div class="ocr-queue-section-title">History</div>
    <div id="ocr-queue"></div>
  `;

  setupUpload(body, panel);
  renderQueue();
}

function setupUpload(body, panel) {
  const zone = document.getElementById('ocr-upload-zone');
  if (!zone) return;

  zone.addEventListener('click', async () => {
    // Use Tauri file dialog if available, fallback to browser input
    if (window.__TAURI_INTERNALS__) {
      try {
        const { open } = await import('@tauri-apps/plugin-dialog');
        const selected = await open({
          multiple: true,
          filters: [{ name: 'PDF', extensions: ['pdf'] }],
        });
        if (selected) {
          const files = Array.isArray(selected) ? selected : [selected];
          for (const path of files) {
            queueFile({ name: path.split('/').pop() || path, path, size: 0 });
          }
        }
      } catch (e) {
        // Fallback: use HTML input
        openBrowserPicker();
      }
    } else {
      openBrowserPicker();
    }
  });

  function openBrowserPicker() {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = '.pdf';
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
        if (file) {
          queueFile({ name: file.name, path: file.path || null, file, size: file.size });
        }
      }
    }

    // Also check for dropped files (some systems use this instead of items)
    if (e.dataTransfer.files.length > 0) {
      for (const file of e.dataTransfer.files) {
        const already = queueItems.find(i => i.filename === file.name && i.timestamp > Date.now() - 1000);
        if (!already) {
          queueFile({ name: file.name, path: file.path || null, file, size: file.size });
        }
      }
    }
  });
}

function queueFile(fileInfo) {
  if (!fileInfo.name.endsWith('.pdf')) {
    notifyProcess('error', 'ocr', { filename: fileInfo.name, reason: 'Not a PDF file' });
    return;
  }
  if (fileInfo.size > 100 * 1024 * 1024) {
    notifyProcess('error', 'ocr', { filename: fileInfo.name, reason: 'File exceeds 100 MB limit' });
    return;
  }

  const item = {
    id: crypto.randomUUID(),
    filename: fileInfo.name,
    path: fileInfo.path || fileInfo.name,
    file: fileInfo.file || null,
    size: fileInfo.size || 0,
    status: 'queued',
    pages: '?',
    chars: 0,
    progress: 0,
    text: '',
    error: null,
    timestamp: Date.now(),
  };
  queueItems.push(item);
  renderQueue();

  // Auto-start processing
  processNextInQueue();
}

async function processNextInQueue() {
  const processing = queueItems.filter(i => i.status === 'processing').length;
  if (processing > 0) return;

  const item = queueItems.find(i => i.status === 'queued');
  if (!item) return;

  item.status = 'processing';
  renderQueue();

  notifyProcess('start', 'ocr', { filename: item.filename, pages: '?', estimate: 'a few minutes' });

  try {
    const ipcFn = getIpc();
    if (ipcFn && item.path) {
      events.emit('status-log', { type: 'info', message: 'OCR: processing ' + item.filename + '...' });
      const result = await invoke('run_ocr', { filePath: item.path });
      item.text = result.text || '';
      item.chars = item.text.length;
      item.pages = result.pages || '?';
      item.status = 'complete';
      item.processId = result.pid || null;
      events.emit('status-log', { type: 'success', message: 'OCR: ' + item.filename + ' done — ' + item.chars.toLocaleString() + ' chars' });
      notifyProcess('complete', 'ocr', { filename: item.filename, chars: item.chars });
    } else {
      // Browser dev mode or no path: simulate
      events.emit('status-log', { type: 'info', message: 'OCR: simulating for ' + item.filename + '...' });
      await new Promise(r => setTimeout(r, 1500));
      item.text = '# Simulated OCR Output\n\nThis is placeholder text for ' + item.filename + '.\n\nIn Tauri mode with Docker services up, the actual OCR pipeline runs via `ocr-helper` and deepseek-ocr.';
      item.chars = item.text.length;
      item.status = 'complete';
      events.emit('status-log', { type: 'success', message: 'OCR: ' + item.filename + ' simulated — ' + item.chars.toLocaleString() + ' chars' });
      notifyProcess('complete', 'ocr', { filename: item.filename, chars: item.chars });
    }
  } catch (e) {
    item.status = 'failed';
    item.error = e.message || String(e);
    notifyProcess('error', 'ocr', { filename: item.filename, reason: item.error });
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
    el.className = 'ocr-queue-item ' + (item.status === 'complete' ? 'complete' : item.status === 'failed' ? 'failed' : '');
    el.innerHTML = `
      <div class="ocr-item-header">
        <span class="ocr-item-name">${item.filename}</span>
        <span class="ocr-item-status ${item.status}">${statusLabel(item.status)}</span>
      </div>
      <div class="ocr-item-meta">${item.pages} pages \u00B7 ${item.chars.toLocaleString()} chars \u00B7 ${timeAgo(item.timestamp)}</div>
      ${item.status === 'processing' ? `
        <div class="ocr-item-progress"><div class="ocr-item-progress-fill" style="width:${item.progress}%"></div></div>
        <div class="ocr-item-actions">
          <button class="ocr-action-btn discard" data-action="cancel" data-id="${item.id}">⏹ Stop</button>
        </div>` : ''}
      ${item.status === 'complete' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="view" data-id="${item.id}">View</button>
          <button class="ocr-action-btn approve" data-action="approve" data-id="${item.id}">Approve</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">Discard</button>
        </div>` : ''}
      ${item.status === 'failed' ? `
        <div class="ocr-item-actions">
          <button class="ocr-action-btn" data-action="retry" data-id="${item.id}">Retry</button>
          <button class="ocr-action-btn discard" data-action="discard" data-id="${item.id}">Discard</button>
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
          events.emit('status-log', { type: 'info', message: 'OCR: cancelling ' + target.filename });
          invoke('cancel_process', { pid: target.processId }).catch(() => {});
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
  notifyProcess('start', 'kb-ingest', { source: item.filename });
  item.status = 'processing';
  item.progress = 0;
  renderQueue();

  if (ipcFn) {
    try {
      // Step 1: Save file record
      item.progress = 10; renderQueue();
      events.emit('status-log', { type: 'info', message: 'Ingest: saving file record...' });
      await invoke('save_file_record', { name: item.filename, path: item.path, chars: item.chars, sourceType: 'ocr' });
      
      // Step 2: Start ingestion pipeline
      item.progress = 20; renderQueue();
      events.emit('status-log', { type: 'info', message: 'Ingest: starting extraction pipeline...' });
      const result = await invoke('ingest_text', { text: item.text, source: item.filename, sourceType: 'ocr' });
      
      // Step 3: Complete
      item.progress = 100; renderQueue();
      notifyProcess('complete', 'kb-ingest', { nodes: '?', edges: '?', facts: '?', source: item.filename });
      events.emit('status-log', { type: 'success', message: 'KB: ' + result });
    } catch (e) {
      console.warn('Ingest failed:', e);
      notifyProcess('error', 'kb-ingest', { reason: e.message || String(e) });
    }
  }

  item.status = 'ingested';
  renderQueue();
  const panel = getPanel('ocr');
  if (panel) renderList(panel);

  // Refresh KB panel (force re-render even if hidden)
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
