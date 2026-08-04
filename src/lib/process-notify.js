// ── Process notification helpers ───────────────────────────

import { events } from './events.js';
import {
  buildProcessAnnouncement,
  createAnnouncementDeduper,
} from './process-announcements.js';

const announcementDeduper = createAnnouncementDeduper();

export function notifyProcess(phase, process, data = {}) {
  events.emit('process-notify', { phase, process, data, timestamp: Date.now() });

  events.emit('status-log', {
    type: phase === 'error' ? 'error' : phase === 'complete' ? 'success' : 'info',
    message: formatStatusMessage(phase, process, data),
  });

  const announcement = buildProcessAnnouncement(phase, process, data);
  if (announcement && announcementDeduper.accept(announcement.dedupeKey)) {
    events.emit('process-announcement', announcement);
  }
}

function formatStatusMessage(phase, process, data) {
  const icons = { start: '\u25B6', progress: '\u25CF', complete: '\u2713', error: '\u2717', cancel: '\u25A0' };
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
      if (phase === 'cancel')   return `${icon} Search cancelled: "${data.query}"`;

    case 'kb-ingest':
      if (phase === 'start')    return `${icon} Adding ${data.source || 'document'} to knowledge base...`;
      if (phase === 'complete' && data.nodes != null) return `${icon} KB updated: +${data.nodes} nodes, +${data.edges} edges, +${data.facts} facts`;
      if (phase === 'complete') return `${icon} KB updated: ${data.source || 'document added'}`;
      if (phase === 'error')    return `${icon} KB ingest failed: ${data.reason}`;

    case 'doc-process':
      if (phase === 'start')    return `${icon} ${data.method || 'Document extraction'}: processing ${data.filename}...`;
      if (phase === 'complete') return `${icon} ${data.filename} ready — ${data.chars || 0} chars, ${data.chunks || 0} chunks`;
      if (phase === 'error')    return `${icon} Document processing failed: ${data.filename} — ${data.reason}`;
      if (phase === 'cancel')   return `${icon} Document processing cancelled: ${data.filename}`;

    case 'graph-analysis':
      if (phase === 'start')    return `${icon} Graph analysis started`;
      if (phase === 'complete') return `${icon} Graph analysis finished: ${data.mergedNodes || 0} merged, ${data.renamedNodes || 0} renamed, ${data.relabeledRelations || 0} relations improved`;
      if (phase === 'error')    return `${icon} Graph analysis failed: ${data.reason}`;
      if (phase === 'cancel')   return `${icon} Graph analysis cancelled`;

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
