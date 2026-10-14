// File metadata, document preview, approval retry, and deletion controls.
import { getPanel } from '../panel-system.js';
import { hasDesktopBackend, invoke, openPdf } from '../../lib/ipc.js';
import { events } from '../../lib/events.js';
import { renderMarkdown } from '../../lib/markdown.js';
import { escapeHtml } from '../../lib/dom.js';

let files = [];

export function renderFilesTab(content) {
  content.innerHTML = '<div style="text-align:center;padding:24px;color:var(--text-muted)">Loading files...</div>';
  if (hasDesktopBackend()) {
    invoke('get_files').then(result => {
      files = Array.isArray(result) ? result : [];
      renderFilesTable(content);
    }).catch(() => { files = []; renderFilesTable(content); });
  } else { files = []; renderFilesTable(content); }
}

function renderFilesTable(content) {
  content.innerHTML = `
    <div style="overflow-y:auto;flex:1"><table class="kb-table">
      <thead><tr><th>File</th><th>Chars</th><th>Source</th><th>Date</th><th>Actions</th></tr></thead>
      <tbody id="kb-files-body"></tbody>
    </table></div>`;
  const tbody = document.getElementById('kb-files-body');
  if (!tbody) return;
  tbody.innerHTML = files.length === 0
    ? '<tr><td colspan="5" style="text-align:center;color:var(--text-muted);padding:32px">No files yet.</td></tr>'
    : files.map(f => {
      const name = escapeHtml(f.name || '?');
      return `<tr>
      <td>${name}${f.status && f.status !== 'committed' ? `<br><small>${escapeHtml(f.status)} · excluded from retrieval</small>` : ''}</td><td>${(f.size||0).toLocaleString()}</td><td>${escapeHtml(f.source || '-')}</td><td>${escapeHtml((f.date || '').slice(0,10))}</td>
      <td style="white-space:nowrap">
        ${f.path?.startsWith('/') ? `<button class="kb-open-btn" data-open-file="${name}">Open</button>` : ''}
        <button class="kb-open-btn" data-view-ocr="${name}">View</button>
        ${f.status === 'failed' ? `<button class="kb-open-btn" data-retry-file="${name}">Retry approval</button>` : ''}
        <button class="kb-delete-btn" data-delete-file="${name}">Delete</button>
      </td></tr>`;
    }).join('');
  tbody.querySelectorAll('[data-open-file]').forEach(btn => {
    btn.addEventListener('click', () => {
      const file = files.find(f => f.name === btn.dataset.openFile);
      if (file?.path) openPdf(file.path);
    });
  });
  tbody.querySelectorAll('[data-view-ocr]').forEach(btn => {
    btn.addEventListener('click', async () => {
      const fileName = btn.dataset.viewOcr;
      const file = files.find(f => f.name === fileName);
      const ocrPanel = getPanel('ocr');
      if (!ocrPanel) return;
      ocrPanel.show();
      const body = ocrPanel.getContentEl();
      body.innerHTML = '<div style="padding:32px;color:var(--text-muted);text-align:center">Loading…</div>';
      try {
        const text = file?.text || await invoke('get_document_text', { name: fileName });
        body.innerHTML = `<button class="ocr-back-btn" id="ocr-kb-back-btn">← Back</button><div class="ocr-preview" style="flex:1;overflow-y:auto;padding:12px">${text ? renderMarkdown(text) : '<em>No text found.</em>'}</div>`;
        document.getElementById('ocr-kb-back-btn').addEventListener('click', () => import('../ocr-panel.js').then(m => m.resetOcrView()));
      } catch (e) {
        body.innerHTML = `<button class="ocr-back-btn" id="ocr-kb-back-btn">← Back</button><div style="padding:12px;color:var(--text-secondary)">Failed: ${escapeHtml(e)}</div>`;
        document.getElementById('ocr-kb-back-btn').addEventListener('click', () => import('../ocr-panel.js').then(m => m.resetOcrView()));
      }
    });
  });
  tbody.querySelectorAll('[data-retry-file]').forEach(btn => {
    btn.addEventListener('click', async () => {
      const file = files.find(f => f.name === btn.dataset.retryFile);
      btn.disabled = true;
      try {
        const text = file.text || await invoke('get_document_text', { name: file.name });
        if (!text) throw new Error('Approval input is unavailable. Start storage and retry, or reprocess the source.');
        await invoke('commit_document', { name: file.name, path: file.path, chars: file.size, sourceType: file.source, text });
        events.emit('kb-refresh');
      } catch (error) { alert('Approval failed: ' + (error.message || error)); }
      finally { btn.disabled = false; }
    });
  });
  tbody.querySelectorAll('[data-delete-file]').forEach(btn => {
    btn.addEventListener('click', () => {
      if (confirm(`Delete "${btn.dataset.deleteFile}" and its graph nodes?`)) {
        invoke('delete_file_record', { name: btn.dataset.deleteFile, deleteNodes: true }).then(msg => {
          events.emit('status-log', { type: 'info', message: msg });
          events.emit('kb-refresh');
        }).catch(e => alert('Failed: ' + e));
      }
    });
  });
}
