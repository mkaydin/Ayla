// ── Meta Search Panel ──────────────────────────────────────

import { createPanel, getPanel } from './panel-system.js';
import { renderMarkdown } from '../lib/markdown.js';
import { invoke, getIpc, openPdf } from '../lib/ipc.js';
import { notifyProcess } from '../lib/process-notify.js';
import { events } from '../lib/events.js';

let searchResults = [];

export function initSearchPanel() {
  const panel = createPanel('search', 'Meta Search', {
    x: Math.round(window.innerWidth * 0.55),
    y: Math.round(window.innerHeight * 0.15),
    width: 600, height: 550,
    minWidth: 480, minHeight: 300,
    className: 'search-panel',
  });

  renderSearchHome(panel);
  return panel;
}

function renderSearchHome(panel) {
  const body = panel.getContentEl();

  const history = searchResults
    .map(s => `
      <div class="search-history-item">
        <span class="search-history-query">${s.query}</span>
        <span class="search-history-meta">${s.papers?.length || 0} papers \u00B7 ${timeAgo(s.timestamp)}</span>
        <button class="search-history-view" data-id="${s.id}">\u25B6 View</button>
      </div>`)
    .join('');

  body.innerHTML = `
    <div style="margin-bottom:var(--space-md)">
      <div class="search-query-label" style="margin-bottom:6px">Search academic literature</div>
      <div style="display:flex;gap:var(--space-sm)">
        <input class="chat-input" id="search-input" placeholder="Enter research topic..." style="flex:1;font-size:var(--font-size-sm)" />
        <button class="search-action-btn primary" id="search-go-btn" style="white-space:nowrap">\uD83D\uDD0D Search</button>
      </div>
    </div>
    ${history ? `<div class="search-history-section">
      <div class="search-history-title">Previous Searches</div>
      ${history}
    </div>` : '<div style="text-align:center;padding:32px;color:var(--text-muted)">No searches yet. Type a topic above.</div>'}
  `;

  // Wire search input
  const searchInput = document.getElementById('search-input');
  const searchBtn = document.getElementById('search-go-btn');

  function doSearch() {
    const q = searchInput.value.trim();
    if (q) runSearch(q);
  }
  searchBtn.addEventListener('click', doSearch);
  searchInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); doSearch(); }
  });

  // Wire history view buttons
  body.querySelectorAll('.search-history-view').forEach(btn => {
    btn.addEventListener('click', () => {
      const id = btn.dataset.id;
      const result = searchResults.find(r => r.id === id);
      if (result) renderReport(panel, result);
    });
  });
}

export async function runSearch(query) {
  const panel = getPanel('search');
  if (!panel) return;

  const body = panel.getContentEl();
  const id = crypto.randomUUID();

  body.innerHTML = `
    <div class="search-query-label">Query</div>
    <div class="search-query-text">${query}</div>
    <div class="search-progress-bar"><div class="search-progress-fill" id="search-progress-fill" style="width:10%"></div></div>
    <div class="search-progress-log" id="search-progress-log">
      <div class="search-log-entry progress">\u25CF Starting meta_search...</div>
    </div>
    <div style="margin-top:var(--space-md)">
      <button class="ocr-action-btn discard" id="search-cancel-btn">\u23F9 Stop Search</button>
    </div>
  `;

  const logEl = document.getElementById('search-progress-log');
  const progressEl = document.getElementById('search-progress-fill');

  function log(msg, cls = '') {
    const div = document.createElement('div');
    div.className = 'search-log-entry ' + cls;
    div.textContent = msg;
    logEl.appendChild(div);
    logEl.scrollTop = logEl.scrollHeight;
  }

  notifyProcess('start', 'search', { query });
  let searchPid = null;

  // Cancel button
  document.getElementById('search-cancel-btn')?.addEventListener('click', async () => {
    if (searchPid) {
      log('\u25A0 Search cancelled by user', 'error');
      await invoke('cancel_process', { pid: searchPid }).catch(() => {});
      const backBtn = document.createElement('button');
      backBtn.className = 'ocr-back-btn';
      backBtn.textContent = '\u2190 All searches';
      backBtn.addEventListener('click', () => renderSearchHome(panel));
      body.appendChild(backBtn);
    }
  });

  try {
    const ipcFn = getIpc();
    if (ipcFn) {
      log('\u25CF Query enriched: generating alternate queries', 'progress');
      if (progressEl) progressEl.style.width = '20%';

      await new Promise(r => setTimeout(r, 500));
      log('\u25CF Searching 8 engines...', 'progress');
      if (progressEl) progressEl.style.width = '35%';

      const result = await invoke('run_search', { query });
      searchPid = result.pid;
      if (progressEl) progressEl.style.width = '100%';

      log('\u2713 Search complete', 'complete');

      const papers = parsed.papers_analyzed || parsed.papers || [];
      const searchResult = {
        id,
        query,
        timestamp: Date.now(),
        papers,
        synthesis: parsed.synthesis || {},
        fullText: result,
      };
      searchResults.unshift(searchResult);

      notifyProcess('complete', 'search', {
        query,
        papers: papers.length,
        themes: parsed.synthesis?.cross_cutting_themes?.length || 0,
      });

      renderReport(panel, searchResult);
    } else {
      log('\u25CF Searching academic databases (simulated)...', 'progress');
      await new Promise(r => setTimeout(r, 2000));
      log('\u2713 Search complete (simulated)', 'complete');

      const searchResult = {
        id, query, timestamp: Date.now(),
        papers: [{ title: 'Simulated Result', authors: 'Dev Mode', year: '2026', key_findings: ['Run in Tauri mode for real searches.'], relevance_score: 0.9 }],
        synthesis: { summary: 'Simulated search. Start Docker services and build `meta_search` binary for real results.', themes: [] },
      };
      searchResults.unshift(searchResult);
      renderReport(panel, searchResult);
    }
  } catch (e) {
    log('\u2717 Search failed: ' + (e.message || String(e)), 'error');
    notifyProcess('error', 'search', { query, reason: e.message || String(e) });
    // Add back button after error
    const backBtn = document.createElement('button');
    backBtn.className = 'ocr-back-btn';
    backBtn.textContent = '\u2190 All searches';
    backBtn.addEventListener('click', () => renderSearchHome(panel));
    body.appendChild(backBtn);
  }
}

function renderReport(panel, result) {
  const body = panel.getContentEl();
  panel.setTitle('Meta Search: ' + result.query.slice(0, 30) + '...');

  const papersHTML = (result.papers || []).map((p, i) => `
    <div class="search-paper-card">
      <div class="search-paper-header">
        <span class="search-paper-rank">#${i + 1}</span>
        <span class="search-paper-title">${p.title || 'Untitled'}</span>
        <span class="search-paper-score">${(p.relevance_score || 0).toFixed(2)}</span>
      </div>
      <div class="search-paper-authors">${p.authors || 'Unknown authors'}, ${p.year || '?'}</div>
      <div class="search-paper-findings">${(p.key_findings || []).slice(0, 3).join('; ') || p.abstract || ''}</div>
      <div class="search-paper-actions">
        ${p.pdf_path ? `<button class="ocr-action-btn" data-pdf="${p.pdf_path}">\uD83D\uDCC4 Open PDF</button>` : ''}
        <button class="ocr-action-btn" data-citation="${encodeURIComponent(p.title || '')}, ${p.authors || ''}, ${p.year || ''}">\uD83D\uDCCB Copy</button>
      </div>
    </div>
  `).join('');

  const synth = result.synthesis || {};
  const synthHTML = synth.summary ? renderMarkdown('## Synthesis\n' + synth.summary) : '';
  const themesHTML = synth.cross_cutting_themes?.length
    ? '## Cross-Cutting Themes\n' + synth.cross_cutting_themes.map((t, i) => `${i + 1}. ${t}`).join('\n')
    : '';

  body.innerHTML = `
    <button class="ocr-back-btn" id="search-back-btn">\u2190 All searches</button>
    <div style="overflow-y:auto;flex:1">
      ${synthHTML}
      ${themesHTML ? renderMarkdown(themesHTML) : ''}
      ${papersHTML ? `<h3 style="font-size:13px;color:var(--text-secondary);margin:16px 0 8px">Papers (${result.papers.length} analyzed)</h3>` + papersHTML : ''}
    </div>
    <div class="search-report-actions">
      <button class="search-action-btn primary" id="search-approve-btn">\u2713 Approve &amp; Add to Knowledge Base</button>
      <button class="search-action-btn" id="search-ocr-btn">\u2192 Send PDFs to OCR</button>
    </div>
  `;

  document.getElementById('search-back-btn').addEventListener('click', () => renderSearchHome(panel));
  document.getElementById('search-approve-btn').addEventListener('click', () => approveSearch(result));
  document.getElementById('search-ocr-btn').addEventListener('click', () => sendPdfsToOcr(result));

  body.querySelectorAll('[data-pdf]').forEach(btn => {
    btn.addEventListener('click', () => openPdf(btn.dataset.pdf));
  });
  body.querySelectorAll('[data-citation]').forEach(btn => {
    btn.addEventListener('click', () => {
      navigator.clipboard.writeText(decodeURIComponent(btn.dataset.citation)).then(() => {
        btn.textContent = 'Copied!';
        setTimeout(() => (btn.textContent = '\uD83D\uDCCB Copy'), 1500);
      });
    });
  });
}

async function approveSearch(result) {
  notifyProcess('start', 'kb-ingest', { source: 'search: ' + result.query });
  const ipcFn = getIpc();
  if (ipcFn) {
    try {
      const ingestResult = await invoke('ingest_text', {
        text: JSON.stringify(result),
        source: result.query,
        sourceType: 'search',
      });
      notifyProcess('complete', 'kb-ingest', { nodes: (result.papers || []).length, edges: '?', facts: '?' });
      events.emit('status-log', { type: 'success', message: 'KB: ' + ingestResult });
      events.emit('kb-refresh');
    } catch (e) {
      notifyProcess('error', 'kb-ingest', { reason: e.message || String(e) });
    }
  } else {
    notifyProcess('complete', 'kb-ingest', { nodes: (result.papers || []).length, edges: '?', facts: '?' });
    events.emit('kb-refresh');
  }
}

function sendPdfsToOcr(result) {
  const ocrPanel = getPanel('ocr');
  if (ocrPanel) ocrPanel.show();
  notifyProcess('start', 'ocr', { filename: (result.papers || []).length + ' PDFs', pages: '?', estimate: 'several minutes' });
}

function timeAgo(ts) {
  const s = Math.floor((Date.now() - ts) / 1000);
  if (s < 60) return 'just now';
  if (s < 3600) return Math.floor(s / 60) + 'm ago';
  if (s < 86400) return Math.floor(s / 3600) + 'h ago';
  return Math.floor(s / 86400) + 'd ago';
}

// Expose for chat panel to call
window.__runSearch = runSearch;
