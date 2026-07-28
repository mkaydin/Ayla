// ── Process notification helpers ───────────────────────────

import { events } from './events.js';

export function notifyProcess(phase, process, data = {}) {
  events.emit('process-notify', { phase, process, data, timestamp: Date.now() });

  events.emit('status-log', {
    type: phase === 'error' ? 'error' : phase === 'complete' ? 'success' : 'info',
    message: formatStatusMessage(phase, process, data),
  });
}

function formatStatusMessage(phase, process, data) {
  const icons = { start: '\u25B6', progress: '\u25CF', complete: '\u2713', error: '\u2717' };
  const icon = icons[phase] || '\u25CF';

  switch (process) {
    case 'ocr':
      if (phase === 'start')    return `${icon} OCR: ${data.filename} (${data.pages} pages)`;
      if (phase === 'progress') return `${icon} OCR: ${data.filename} page ${data.current}/${data.total}`;
      if (phase === 'complete') return `${icon} OCR done: ${data.filename} (${data.chars} chars)`;
      if (phase === 'error')    return `${icon} OCR failed: ${data.filename} \u2014 ${data.reason}`;

    case 'search':
      if (phase === 'start')    return `${icon} Search: "${data.query}"`;
      if (phase === 'progress') return `${icon} Search: found ${data.results}, analyzing ${data.current}/${data.total}`;
      if (phase === 'complete') return `${icon} Search done: ${data.papers} papers, ${data.themes} themes`;
      if (phase === 'error')    return `${icon} Search error: ${data.reason}`;

    case 'kb-ingest':
      if (phase === 'complete') return `${icon} KB updated: +${data.nodes} nodes, +${data.edges} edges, +${data.facts} facts`;
      if (phase === 'error')    return `${icon} KB ingest failed: ${data.reason}`;

    case 'model':
      if (phase === 'error')    return `${icon} Model offline: ${data.model}`;
      if (phase === 'complete') return `${icon} Model ready: ${data.model}`;

    case 'memory':
      if (phase === 'error')    return `${icon} DB offline: ${data.db}`;
      if (phase === 'complete') return `${icon} DB reconnected: ${data.db}`;

    default:
      return `${icon} ${data.message || 'Unknown process event'}`;
  }
}
