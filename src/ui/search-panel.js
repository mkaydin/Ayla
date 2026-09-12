// ── Meta Search Panel ──────────────────────────────────────
// Persistent topic history + two gathering modes:
// 1. General knowledge document
// 2. Scientific paper/PDF collection

import { createPanel, getPanel } from './panel-system.js';
import { renderMarkdown } from '../lib/markdown.js';
import {
  invoke,
  invokeCancelable,
  hasDesktopBackend,
  listenEvent,
  openPdf,
} from '../lib/ipc.js';
import { notifyProcess } from '../lib/process-notify.js';
import { events } from '../lib/events.js';
import { escapeHtml } from '../lib/dom.js';

let searchResults = [];
let activeMode = 'knowledge';
let historyLoaded = false;
let batchRunning = false;
let notificationListenerInstalled = false;

const MODES = {
  knowledge: {
    label: 'Knowledge document',
    icon: '\uD83E\uDDE0',
    description: 'Gather multiple sources and create a structured, KB-ready general knowledge document.',
    placeholder: 'Topic to research and explain...',
  },
  papers: {
    label: 'Scientific papers',
    icon: '\uD83D\uDCD1',
    description: 'Search academic sources and collect downloadable scientific paper PDFs.',
    placeholder: 'Scientific topic, method, or research question...',
  },
};

export function initSearchPanel() {
  const panel = createPanel('search', 'Research & Meta Search', {
    x: Math.round(window.innerWidth * 0.55),
    y: Math.round(window.innerHeight * 0.15),
    width: 660, height: 610,
    minWidth: 500, minHeight: 360,
    className: 'search-panel',
  });

  renderSearchHome(panel);
  loadSearchHistory(panel);
  if (!notificationListenerInstalled) {
    notificationListenerInstalled = true;
    events.on('notification-open', ({ target, operationId }) => {
      if (target !== 'search') return;
      const searchPanel = getPanel('search');
      if (!searchPanel) return;
      const result = searchResults.find(item => item.id === operationId || item.searchId === operationId);
      if (result) renderReport(searchPanel, result);
    });
  }
  return panel;
}

async function loadSearchHistory(panel) {
  if (!hasDesktopBackend()) {
    historyLoaded = true;
    renderSearchHome(panel);
    return;
  }
  try {
    const rows = await invoke('get_search_history');
    searchResults = (Array.isArray(rows) ? rows : []).map(normalizeHistoryResult);
  } catch (error) {
    events.emit('status-log', {
      type: 'error',
      message: 'Meta Search history: ' + errorMessage(error),
    });
  } finally {
    historyLoaded = true;
    if (!panel.getContentEl().querySelector('.search-progress-log')) {
      renderSearchHome(panel);
    }
  }
}

function normalizeHistoryResult(value) {
  return {
    ...value,
    id: value.search_id || value.id || crypto.randomUUID(),
    searchId: value.search_id || value.id || '',
    query: value.query || 'Untitled search',
    mode: value.mode === 'papers' ? 'papers' : 'knowledge',
    timestamp: parseTimestamp(value.timestamp),
    papers: Array.isArray(value.papers) ? value.papers : [],
    documents: Array.isArray(value.documents) ? value.documents : [],
    synthesis: value.synthesis || {},
    knowledgeDocument: value.knowledge_document || value.knowledgeDocument || '',
    knowledgeIngested: Boolean(value.knowledge_ingested || value.knowledgeIngested),
    persisted: value.persisted !== false,
    status: value.status || 'complete',
    warnings: Array.isArray(value.warnings) ? value.warnings : [],
  };
}

function normalizeLiveResult(value, query, mode, fallbackId) {
  return normalizeHistoryResult({
    ...value,
    id: value.search_id || value.session_id || fallbackId,
    search_id: value.search_id || value.session_id || fallbackId,
    query,
    mode,
    timestamp: Date.now(),
    papers: value.papers_analyzed || value.papers || [],
    documents: value.documents || [],
    knowledge_document: value.knowledge_document || '',
    synthesis: value.synthesis || {},
  });
}

function renderSearchHome(panel) {
  panel.setTitle('Research & Meta Search');
  const body = panel.getContentEl();
  const mode = MODES[activeMode];
  const backendAvailable = hasDesktopBackend();

  const history = searchResults.map(result => {
    const processed = result.documents.filter(document => document.status === 'processed' || document.status === 'ingested').length;
    const modeInfo = MODES[result.mode] || MODES.knowledge;
    return `
      <article class="search-history-item">
        <div class="search-history-main">
          <div class="search-history-query">${escapeHtml(result.query)}</div>
          <div class="search-history-meta">
            <span class="search-mode-badge">${modeInfo.icon} ${escapeHtml(modeInfo.label)}</span>
            <span>${result.papers.length} sources</span>
            <span>${result.documents.length} PDFs</span>
            ${processed ? `<span>${processed} processed</span>` : ''}
            <span>${timeAgo(result.timestamp)}</span>
          </div>
        </div>
        <button class="search-history-view" data-search-id="${escapeHtml(result.id)}">\u25B6 View</button>
      </article>`;
  }).join('');

  body.innerHTML = `
    <section class="search-compose">
      <div class="search-section-label">Gathering mode</div>
      <div class="search-mode-grid">
        ${Object.entries(MODES).map(([key, item]) => `
          <button class="search-mode-card ${activeMode === key ? 'active' : ''}" data-mode="${key}">
            <span class="search-mode-icon">${item.icon}</span>
            <span class="search-mode-copy">
              <strong>${escapeHtml(item.label)}</strong>
              <small>${escapeHtml(item.description)}</small>
            </span>
          </button>
        `).join('')}
      </div>
      <div class="search-query-row">
        <input class="chat-input search-topic-input" id="search-input"
          placeholder="${escapeHtml(mode.placeholder)}" />
        <button class="search-action-btn primary" id="search-go-btn" ${backendAvailable ? '' : 'disabled'}>
          ${backendAvailable ? '\uD83D\uDD0D Gather' : 'Desktop backend required'}
        </button>
      </div>
    </section>
    <section class="search-history-section">
      <div class="search-history-title">
        <span>Topic history</span>
        <span>${historyLoaded ? searchResults.length : 'Loading\u2026'}</span>
      </div>
      <div class="search-history-list">
        ${history || (historyLoaded
          ? '<div class="search-empty-state">No saved searches yet.</div>'
          : '<div class="search-empty-state">Loading history from SurrealDB\u2026</div>')}
      </div>
    </section>
  `;

  body.querySelectorAll('.search-mode-card').forEach(button => {
    button.addEventListener('click', () => {
      activeMode = button.dataset.mode;
      renderSearchHome(panel);
      body.querySelector('#search-input')?.focus();
    });
  });

  const searchInput = body.querySelector('#search-input');
  const searchButton = body.querySelector('#search-go-btn');
  const start = () => {
    const query = searchInput.value.trim();
    if (query) runSearch(query, activeMode);
  };
  searchButton?.addEventListener('click', start);
  searchInput?.addEventListener('keydown', event => {
    if (event.key === 'Enter') {
      event.preventDefault();
      start();
    }
  });

  body.querySelectorAll('.search-history-view').forEach(button => {
    button.addEventListener('click', () => {
      const result = searchResults.find(item => item.id === button.dataset.searchId);
      if (result) renderReport(panel, result);
    });
  });
}

export async function runSearch(query, mode = activeMode) {
  const panel = getPanel('search');
  if (!panel) return;

  mode = mode === 'papers' ? 'papers' : 'knowledge';
  activeMode = mode;
  const body = panel.getContentEl();
  const fallbackId = crypto.randomUUID();
  const modeInfo = MODES[mode];

  body.innerHTML = `
    <div class="search-query-label">${modeInfo.icon} ${escapeHtml(modeInfo.label)}</div>
    <div class="search-query-text">${escapeHtml(query)}</div>
    <div class="search-progress-bar"><div class="search-progress-fill" id="search-progress-fill" style="width:10%"></div></div>
    <div class="search-progress-log" id="search-progress-log">
      <div class="search-log-entry progress">\u25CF Starting Meta Search\u2026</div>
    </div>
    <div class="search-progress-actions">
      <button class="ocr-action-btn discard" id="search-cancel-btn">\u23F9 Stop Search</button>
    </div>
  `;

  const logElement = body.querySelector('#search-progress-log');
  const progressElement = body.querySelector('#search-progress-fill');
  const log = (message, className = '') => {
    const entry = document.createElement('div');
    entry.className = 'search-log-entry ' + className;
    entry.textContent = message;
    logElement.appendChild(entry);
    logElement.scrollTop = logElement.scrollHeight;
  };

  notifyProcess('start', 'search', { query, operationId: fallbackId });
  let searchPid = null;
  let searchOperation = null;
  let cancelRequested = false;
  let unlistenStarted = () => {};

  try {
    unlistenStarted = await listenEvent('search-started', event => {
      if (
        event.payload?.requestId
        && event.payload.requestId !== searchOperation?.requestId
      ) return;
      if (event.payload?.pid) {
        searchPid = event.payload.pid;
        searchOperation?.setProcessId(searchPid);
      }
    });

    body.querySelector('#search-cancel-btn')?.addEventListener('click', async () => {
      if (cancelRequested) return;
      cancelRequested = true;
      const button = body.querySelector('#search-cancel-btn');
      if (button) {
        button.disabled = true;
        button.textContent = '\u23F3 Cancelling\u2026';
      }
      await searchOperation?.cancel().catch(() => false);
      log('\u25A0 Cancelling search\u2026', 'error');
    });

    if (!hasDesktopBackend()) {
      throw new Error('Research requires the Rust desktop backend');
    }
    log('\u25CF Enriching topic and querying academic/web sources\u2026', 'progress');
    progressElement.style.width = '25%';
    searchOperation = invokeCancelable('run_search', { query, mode });
    if (searchPid) searchOperation.setProcessId(searchPid);
    if (cancelRequested) await searchOperation.cancel().catch(() => false);
    const result = await searchOperation.promise;
    searchPid = result.pid || searchPid;
    progressElement.style.width = '100%';
    log(`\u2713 Collected ${(result.papers_analyzed || []).length} sources and ${(result.documents || []).length} PDFs`, 'complete');

    const searchResult = normalizeLiveResult(result, query, mode, fallbackId);
    searchResults = [
      searchResult,
      ...searchResults.filter(item => item.id !== searchResult.id),
    ];

    if (result.persistence_error) {
      events.emit('status-log', {
        type: 'error',
        message: 'Search completed, but SurrealDB persistence failed: ' + result.persistence_error,
      });
    }
    notifyProcess('complete', 'search', {
      query,
      papers: searchResult.papers.length,
      themes: searchResult.synthesis?.cross_cutting_themes?.length || 0,
      operationId: searchResult.id,
    });
    if (searchResult.status === 'partial' || searchResult.warnings.length) events.emit('status-log', { type: 'info', message: `Partial research results: ${searchResult.warnings.join('; ')}` });
    renderReport(panel, searchResult);
  } catch (error) {
    const cancelled = cancelRequested || isCancellationError(error);
    const reason = cancelled ? 'Cancelled by user' : errorMessage(error);
    log(cancelled ? '\u25A0 Search cancelled' : '\u2717 Search failed: ' + reason, 'error');
    notifyProcess(cancelled ? 'cancel' : 'error', 'search', {
      query,
      reason,
      operationId: fallbackId,
    });
    const back = document.createElement('button');
    back.className = 'ocr-back-btn';
    back.textContent = '\u2190 Topic history';
    back.addEventListener('click', () => renderSearchHome(panel));
    body.appendChild(back);
  } finally {
    try { unlistenStarted(); } catch {}
  }
}

function renderReport(panel, result) {
  const body = panel.getContentEl();
  const modeInfo = MODES[result.mode] || MODES.knowledge;
  panel.setTitle(result.query.length > 34 ? result.query.slice(0, 34) + '\u2026' : result.query);

  const synthesis = result.synthesis || {};
  const knowledgeDocument = result.knowledgeDocument
    || `# General Knowledge: ${result.query}\n\n${synthesis.summary || 'No synthesis available.'}`;
  const papersHtml = result.papers.map((paper, index) => renderPaperCard(paper, index)).join('');
  const documentsHtml = result.documents.map(document => renderDocumentCard(document)).join('');
  const processedCount = result.documents.filter(document => ['processed', 'ingested'].includes(document.status)).length;

  body.innerHTML = `
    ${result.status === 'partial' || result.warnings?.length ? `<div class="search-state-pill">Partial results: ${escapeHtml((result.warnings || []).join('; '))}</div>` : ''}
    <div class="search-report-header">
      <button class="ocr-back-btn" id="search-back-btn">\u2190 Topic history</button>
      <div class="search-report-heading">
        <span class="search-mode-badge">${modeInfo.icon} ${escapeHtml(modeInfo.label)}</span>
        <strong>${escapeHtml(result.query)}</strong>
        <small>
          ${result.papers.length} analyzed sources \u00B7 ${result.documents.length} PDFs
          ${result.persisted ? '\u00B7 Saved in SurrealDB' : '\u00B7 Not persisted'}
        </small>
      </div>
    </div>
    <div class="search-report-scroll">
      <section class="search-result-section">
        <div class="search-section-heading">
          <span>KB-ready knowledge document</span>
          <span class="search-state-pill ${result.knowledgeIngested ? 'success' : ''}">
            ${result.knowledgeIngested ? 'In knowledge base' : 'Ready'}
          </span>
        </div>
        <div class="search-knowledge-preview">${renderMarkdown(knowledgeDocument)}</div>
        <div class="search-inline-actions">
          <button class="search-action-btn primary" id="search-knowledge-ingest"
            ${result.knowledgeIngested || batchRunning ? 'disabled' : ''}>
            ${result.knowledgeIngested ? '\u2713 Added to KB' : '\u2713 Add knowledge document to KB'}
          </button>
        </div>
      </section>

      <section class="search-result-section">
        <div class="search-section-heading">
          <span>Downloaded documents</span>
          <span>${processedCount}/${result.documents.length} processed</span>
        </div>
        ${documentsHtml || '<div class="search-empty-state">No PDFs were downloaded for this topic.</div>'}
        ${result.documents.length ? `
          <div class="search-batch-actions">
            <div>
              <strong>Integrate documents into the knowledge base</strong>
              <small>Processing results are saved to SurrealDB before ingestion.</small>
            </div>
            <div class="search-inline-actions">
              <button class="search-action-btn primary" id="search-quick-all" ${batchRunning ? 'disabled' : ''}>
                \u26A1 Quick Extract all \u2192 KB
              </button>
              <button class="search-action-btn" id="search-ocr-all" ${batchRunning ? 'disabled' : ''}>
                \uD83D\uDD6E OCR all PDFs \u2192 KB
              </button>
            </div>
          </div>` : ''}
      </section>

      ${papersHtml ? `
        <section class="search-result-section">
          <div class="search-section-heading">
            <span>Analyzed sources</span>
            <span>${result.papers.length}</span>
          </div>
          ${papersHtml}
        </section>` : ''}
    </div>
  `;

  wireReportActions(panel, result);
}

function renderPaperCard(paper, index) {
  const abstractText = paper.abstract_text || paper.abstract || '';
  const title = paper.title?.trim() || firstUsefulLine(abstractText) || 'Untitled source';
  const year = paper.year ?? '?';
  const score = Number(paper.relevance_score || 0);
  return `
    <article class="search-paper-card">
      <div class="search-paper-header">
        <span class="search-paper-rank">#${index + 1}</span>
        <span class="search-paper-title">${escapeHtml(title)}</span>
        <span class="search-paper-score">${score.toFixed(2)}</span>
      </div>
      <div class="search-paper-authors">${escapeHtml(paper.authors || 'Unknown authors')} \u00B7 ${escapeHtml(year)}</div>
      <div class="search-paper-findings">${escapeHtml(
        (paper.key_findings || []).slice(0, 3).join('; ')
        || abstractText.slice(0, 420)
        || paper.relevance_rationale
        || 'No extracted summary.',
      )}</div>
      <div class="search-paper-actions">
        ${paper.source_url ? `<button class="ocr-action-btn" data-copy-paper="${index}">\uD83D\uDCCB Copy citation</button>` : ''}
      </div>
    </article>`;
}

function renderDocumentCard(document) {
  const status = document.status || 'downloaded';
  const method = document.extraction_method || '';
  const isBusy = status === 'processing';
  const isProcessed = status === 'processed' || status === 'ingested';
  return `
    <article class="search-document-card ${escapeHtml(status)}">
      <div class="search-document-icon">\uD83D\uDCD5</div>
      <div class="search-document-info">
        <strong title="${escapeHtml(document.path || '')}">${escapeHtml(document.name || 'paper.pdf')}</strong>
        <small>
          ${formatBytes(Number(document.size_bytes || 0))}
          ${method ? `\u00B7 ${method === 'ocr' ? 'OCR' : 'Quick Extract'}` : ''}
          ${document.chars ? `\u00B7 ${Number(document.chars).toLocaleString()} chars` : ''}
        </small>
      </div>
      <span class="search-state-pill ${status === 'ingested' ? 'success' : isBusy ? 'busy' : ''}">
        ${documentStatusLabel(status)}
      </span>
      <div class="search-document-actions">
        <button class="ocr-action-btn" data-doc-action="open" data-doc-id="${escapeHtml(document.document_id)}">Open</button>
        ${isBusy ? `
          <button class="ocr-action-btn discard" data-doc-action="cancel" data-doc-id="${escapeHtml(document.document_id)}"
            ${document.cancelRequested ? 'disabled' : ''}>
            ${document.cancelRequested ? '\u23F3 Cancelling\u2026' : '\u23F9 Stop'}
          </button>` : ''}
        <button class="ocr-action-btn" data-doc-action="quick" data-doc-id="${escapeHtml(document.document_id)}"
          ${isBusy || batchRunning ? 'disabled' : ''}>\u26A1 Quick</button>
        <button class="ocr-action-btn" data-doc-action="ocr" data-doc-id="${escapeHtml(document.document_id)}"
          ${isBusy || batchRunning ? 'disabled' : ''}>\uD83D\uDD6E OCR</button>
        ${isProcessed ? `
          <button class="ocr-action-btn" data-doc-action="view" data-doc-id="${escapeHtml(document.document_id)}">View text</button>
          <button class="ocr-action-btn approve" data-doc-action="ingest" data-doc-id="${escapeHtml(document.document_id)}"
            ${status === 'ingested' || batchRunning ? 'disabled' : ''}>
            ${status === 'ingested' ? '\u2713 In KB' : 'Add to KB'}
          </button>` : ''}
      </div>
      ${document.error ? `<div class="search-document-error">${escapeHtml(document.error)}</div>` : ''}
    </article>`;
}

function wireReportActions(panel, result) {
  const body = panel.getContentEl();
  body.querySelector('#search-back-btn')?.addEventListener('click', () => renderSearchHome(panel));
  body.querySelector('#search-knowledge-ingest')?.addEventListener('click', () => ingestKnowledgeDocument(panel, result));
  body.querySelector('#search-quick-all')?.addEventListener('click', () => processAllDocuments(panel, result, 'quick_extract'));
  body.querySelector('#search-ocr-all')?.addEventListener('click', () => processAllDocuments(panel, result, 'ocr'));

  body.querySelectorAll('[data-copy-paper]').forEach(button => {
    button.addEventListener('click', async () => {
      const paper = result.papers[Number(button.dataset.copyPaper)];
      const citation = [
        paper?.title || firstUsefulLine(paper?.abstract_text || ''),
        paper?.authors,
        paper?.year,
        paper?.source_url,
      ].filter(Boolean).join(', ');
      await navigator.clipboard.writeText(citation);
      button.textContent = 'Copied!';
      setTimeout(() => { button.textContent = '\uD83D\uDCCB Copy citation'; }, 1200);
    });
  });

  body.querySelectorAll('[data-doc-action]').forEach(button => {
    button.addEventListener('click', async () => {
      const document = result.documents.find(item => item.document_id === button.dataset.docId);
      if (!document) return;
      const action = button.dataset.docAction;
      if (action === 'open') return openPdf(document.path);
      if (action === 'view') return renderDocumentPreview(panel, result, document);
      if (action === 'ingest') return ingestSearchDocument(panel, result, document);
      if (action === 'quick') return processSearchDocument(panel, result, document, 'quick_extract');
      if (action === 'ocr') return processSearchDocument(panel, result, document, 'ocr');
      if (action === 'cancel') {
        document.cancelRequested = true;
        void document.operation?.cancel().catch(() => false);
        renderReport(panel, result);
      }
    });
  });
}

async function ingestKnowledgeDocument(panel, result) {
  const text = result.knowledgeDocument;
  if (!text) return;
  const filename = `General Knowledge - ${safeFilename(result.query)}.md`;
  const announcementData = {
    kind: 'topic',
    topic: result.query,
    source: filename,
    target: 'search',
    operationId: result.id || result.searchId,
  };
  notifyProcess('start', 'kb-ingest', announcementData);
  try {
    if (!hasDesktopBackend()) throw new Error('Knowledge ingestion requires the Rust desktop backend');
    await invoke('commit_document', {
      name: filename,
      path: `search://${result.searchId}/knowledge.md`,
      chars: text.length,
      sourceType: 'meta_search_knowledge',
      text,
    });
    result.knowledgeIngested = true;
    notifyProcess('complete', 'kb-ingest', announcementData);
    events.emit('status-log', { type: 'success', message: `KB: added ${filename}` });
    events.emit('kb-refresh');
    try {
      await invoke('mark_search_knowledge_ingested', { searchId: result.searchId });
    } catch (error) {
      events.emit('status-log', {
        type: 'warning',
        message: `KB ingest succeeded; search-history status update will need retry: ${errorMessage(error)}`,
      });
    }
  } catch (error) {
    notifyProcess('error', 'kb-ingest', { ...announcementData, reason: errorMessage(error) });
    events.emit('status-log', { type: 'error', message: 'Knowledge document: ' + errorMessage(error) });
  }
  renderReport(panel, result);
}

async function processAllDocuments(panel, result, method) {
  if (batchRunning) return;
  batchRunning = true;
  renderReport(panel, result);
  try {
    for (const document of result.documents) {
      if (method === 'ocr' && !document.name?.toLowerCase().endsWith('.pdf')) continue;
      const processed = await processSearchDocument(panel, result, document, method);
      if (processed) await ingestSearchDocument(panel, result, document);
    }
  } finally {
    batchRunning = false;
    renderReport(panel, result);
  }
}

async function processSearchDocument(panel, result, document, method, rerender = true) {
  document.status = 'processing';
  document.error = '';
  document.cancelRequested = false;
  if (rerender) renderReport(panel, result);
  const label = method === 'ocr' ? 'OCR' : 'Quick Extract';
  notifyProcess('start', 'doc-process', {
    filename: document.name,
    method: label,
    operationId: document.document_id,
  });
  events.emit('status-log', { type: 'info', message: `${label}: processing ${document.name}\u2026` });

  let unlistenStarted = () => {};
  try {
    if (!hasDesktopBackend()) throw new Error('Document processing requires the Rust desktop backend');
    if (method === 'ocr') {
      unlistenStarted = await listenEvent('ocr-started', event => {
        if (
          event.payload?.requestId
          && event.payload.requestId !== document.operation?.requestId
        ) return;
        if (event.payload?.pid) document.operation?.setProcessId(event.payload.pid);
      });
    }
    document.operation = invokeCancelable(
      method === 'ocr' ? 'run_ocr' : 'run_quick_extract',
      { filePath: document.path },
    );
    if (document.cancelRequested) await document.operation.cancel().catch(() => false);
    const response = await document.operation.promise;
    const text = response.text || '';
    const chars = Number(response.chars || text.length);
    const chunks = Number(response.chunks || 0);

    await invoke('save_search_document_result', {
      searchId: result.searchId,
      documentId: document.document_id,
      method,
      text,
      chars,
      chunks,
      ocrOperationId: method === 'ocr' ? response.pid : null,
    });

    document.status = 'processed';
    document.extraction_method = method;
    document.ocr_operation_id = method === 'ocr' ? response.pid : null;
    document.chars = chars;
    document.chunks = chunks;
    document.extractedText = text;
    notifyProcess('complete', 'doc-process', {
      filename: document.name,
      method: label,
      chars,
      chunks,
      operationId: document.document_id,
    });
    events.emit('status-log', {
      type: 'success',
      message: `${label}: ${document.name} saved to SurrealDB \u2014 ${chars.toLocaleString()} chars`,
    });
    return true;
  } catch (error) {
    document.status = 'failed';
    document.error = document.cancelRequested || isCancellationError(error)
      ? 'Cancelled by user'
      : errorMessage(error);
    const cancelled = document.cancelRequested || isCancellationError(error);
    notifyProcess(cancelled ? 'cancel' : 'error', 'doc-process', {
      filename: document.name,
      method: label,
      reason: document.error,
      operationId: document.document_id,
    });
    return false;
  } finally {
    try { unlistenStarted(); } catch {}
    delete document.operation;
    document.cancelRequested = false;
    if (rerender) renderReport(panel, result);
  }
}

async function ingestSearchDocument(panel, result, document, rerender = true) {
  try {
    let text = document.extractedText || '';
    if (!text && hasDesktopBackend()) {
      const stored = await invoke('get_search_document_content', {
        searchId: result.searchId,
        documentId: document.document_id,
      });
      text = stored.extracted_text || '';
      document.ocr_operation_id = stored.ocr_operation_id;
    }
    if (!text) throw new Error('Process this document with Quick Extract or OCR first');

    const method = document.extraction_method || 'quick_extract';
    const kbName = `${safeFilename(result.query)} - ${document.name}`;
    const announcementData = {
      kind: 'document',
      source: kbName,
      target: 'search',
      operationId: document.document_id,
    };
    notifyProcess('start', 'kb-ingest', announcementData);
    if (method === 'ocr') {
      if (!document.ocr_operation_id) throw new Error('Reprocess this older OCR result to approve its structured evidence');
      await invoke('ingest_ocr_knowledge', { operationId: document.ocr_operation_id });
    } else {
    await invoke('commit_document', {
      name: kbName,
      path: document.path,
      chars: text.length,
      sourceType: `search_${method}`,
      text,
    });
    }
    document.status = 'ingested';
    document.extractedText = text;
    notifyProcess('complete', 'kb-ingest', announcementData);
    events.emit('status-log', { type: 'success', message: `KB: added ${kbName}` });
    events.emit('kb-refresh');
    try {
      await invoke('mark_search_document_ingested', {
        searchId: result.searchId,
        documentId: document.document_id,
      });
    } catch (error) {
      events.emit('status-log', {
        type: 'warning',
        message: `KB ingest succeeded; document-history status update will need retry: ${errorMessage(error)}`,
      });
    }
    return true;
  } catch (error) {
    notifyProcess('error', 'kb-ingest', {
      kind: 'document',
      source: `${safeFilename(result.query)} - ${document.name}`,
      target: 'search',
      operationId: document.document_id,
      reason: errorMessage(error),
    });
    document.error = errorMessage(error);
    return false;
  } finally {
    if (rerender) renderReport(panel, result);
  }
}

async function renderDocumentPreview(panel, result, document) {
  const body = panel.getContentEl();
  try {
    let text = document.extractedText || '';
    if (!text && hasDesktopBackend()) {
      const stored = await invoke('get_search_document_content', {
        searchId: result.searchId,
        documentId: document.document_id,
      });
      text = stored.extracted_text || '';
      document.extractedText = text;
    }
    body.innerHTML = `
      <button class="ocr-back-btn" id="search-document-back">\u2190 Search report</button>
      <div class="search-document-preview-header">
        <strong>${escapeHtml(document.name)}</strong>
        <small>${escapeHtml(document.extraction_method || '')} \u00B7 ${Number(document.chars || text.length).toLocaleString()} chars</small>
      </div>
      <div class="ocr-preview search-document-preview">${renderMarkdown(text || 'No processed text stored.')}</div>
    `;
    body.querySelector('#search-document-back')?.addEventListener('click', () => renderReport(panel, result));
  } catch (error) {
    document.error = errorMessage(error);
    renderReport(panel, result);
  }
}

function documentStatusLabel(status) {
  return {
    downloaded: 'Downloaded',
    processing: 'Processing\u2026',
    processed: 'Processed',
    ingested: 'In KB',
    failed: 'Failed',
  }[status] || status;
}

function firstUsefulLine(text) {
  return String(text || '').split('\n').map(line => line.trim()).find(Boolean) || '';
}

function safeFilename(value) {
  return String(value || 'topic')
    .replace(/[\\/:*?"<>|]/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, 90) || 'topic';
}

function parseTimestamp(value) {
  if (typeof value === 'number') return value;
  const parsed = Date.parse(value || '');
  return Number.isFinite(parsed) ? parsed : Date.now();
}

function formatBytes(bytes) {
  if (!bytes) return 'Size unknown';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function timeAgo(timestamp) {
  const seconds = Math.max(0, Math.floor((Date.now() - timestamp) / 1000));
  if (seconds < 60) return 'just now';
  if (seconds < 3600) return Math.floor(seconds / 60) + 'm ago';
  if (seconds < 86400) return Math.floor(seconds / 3600) + 'h ago';
  return Math.floor(seconds / 86400) + 'd ago';
}

function errorMessage(error) {
  return error?.message || String(error);
}

function isCancellationError(error) {
  return error?.code === 'request_cancelled'
    || /\bcancel(?:led|ed)?\b/i.test(errorMessage(error));
}

// Expose for chat-panel tool routing.
window.__runSearch = runSearch;
