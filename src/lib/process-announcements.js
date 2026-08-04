const DEFAULT_TTL_MS = 5_000;

export function cleanAnnouncementLabel(value, fallback = 'this task', maxLength = 100) {
  const cleaned = String(value || '')
    .replace(/[\u0000-\u001f\u007f]/g, ' ')
    .replace(/[`*_~#<>|\\[\]{}$]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
  return (cleaned || fallback).slice(0, maxLength);
}

function methodKind(value) {
  return /ocr/i.test(String(value || '')) ? 'ocr' : 'quick';
}

function notification(title, body, target, urgency = 'normal') {
  return { title, body, target, urgency };
}

export function buildProcessAnnouncement(phase, process, data = {}) {
  if (phase === 'progress' || data.silentAnnouncement) return null;

  const operationId = String(data.operationId || data.requestId || '').slice(0, 80);
  let speech = '';
  let desktop = null;
  const priority = phase === 'error' ? 100 : phase === 'complete' ? 80 : 60;

  if (process === 'search') {
    const topic = cleanAnnouncementLabel(data.query, 'Topic');
    if (phase === 'start') speech = `Searching for ${topic} papers.`;
    if (phase === 'complete') {
      speech = `${topic} search complete.`;
      desktop = notification('Research complete', `${topic} — ${Number(data.papers || 0)} papers, ${Number(data.themes || 0)} themes`, 'search');
    }
    if (phase === 'error') {
      speech = `${topic} search failed.`;
      desktop = notification('Research failed', `${topic} — ${cleanAnnouncementLabel(data.reason, 'Unknown error', 180)}`, 'search', 'critical');
    }
    if (phase === 'cancel') speech = `${topic} search cancelled.`;
  }

  if (process === 'doc-process') {
    const kind = methodKind(data.method);
    const filename = cleanAnnouncementLabel(data.filename, 'Document', 120);
    const spokenName = kind === 'ocr' ? 'OCR process' : 'Quick extraction';
    if (phase === 'start') speech = `${spokenName} started.`;
    if (phase === 'complete') {
      speech = `${spokenName} finished.`;
      desktop = notification(`${spokenName} finished`, `${filename} — ${Number(data.chars || 0).toLocaleString()} characters`, 'ocr');
    }
    if (phase === 'error') {
      speech = `${spokenName} failed.`;
      desktop = notification(`${spokenName} failed`, `${filename} — ${cleanAnnouncementLabel(data.reason, 'Unknown error', 180)}`, 'ocr', 'critical');
    }
    if (phase === 'cancel') speech = `${spokenName} cancelled.`;
  }

  if (process === 'graph-analysis') {
    if (phase === 'start') speech = 'Graph analysis started.';
    if (phase === 'complete') {
      speech = 'Graph analysis finished.';
      desktop = notification(
        'Graph analysis finished',
        `${Number(data.mergedNodes || 0)} merged, ${Number(data.renamedNodes || 0)} renamed, ${Number(data.updatedMeanings || 0)} meanings improved, ${Number(data.updatedNodeTypes || 0)} types corrected, ${Number(data.relabeledRelations || 0)} relations improved`,
        'kb',
      );
    }
    if (phase === 'error') {
      speech = 'Graph analysis failed.';
      desktop = notification('Graph analysis failed', cleanAnnouncementLabel(data.reason, 'Unknown error', 180), 'kb', 'critical');
    }
    if (phase === 'cancel') speech = 'Graph analysis cancelled.';
  }

  if (process === 'kb-ingest') {
    const isTopic = data.kind === 'topic';
    const topic = cleanAnnouncementLabel(data.topic, 'Topic');
    const source = cleanAnnouncementLabel(data.source, 'Document', 120);
    const subject = isTopic ? topic : 'document';
    if (phase === 'start') {
      speech = isTopic
        ? `Adding ${topic} to knowledge base.`
        : 'Adding document to knowledge.';
    }
    if (phase === 'complete') {
      speech = isTopic
        ? `${topic} added to knowledge base.`
        : 'Document added to knowledge.';
      desktop = notification(
        'Knowledge base updated',
        isTopic ? `${topic} was added to the knowledge base` : `${source} was added to the knowledge base`,
        data.target === 'ocr' ? 'ocr' : 'search',
      );
    }
    if (phase === 'error') {
      speech = `Adding ${subject} to knowledge base failed.`;
      desktop = notification(
        'Knowledge base update failed',
        `${isTopic ? topic : source} — ${cleanAnnouncementLabel(data.reason, 'Unknown error', 180)}`,
        data.target === 'ocr' ? 'ocr' : 'search',
        'critical',
      );
    }
    if (phase === 'cancel') speech = `Adding ${subject} to knowledge base cancelled.`;
  }

  if (!speech && !desktop) return null;
  const identity = operationId || cleanAnnouncementLabel(data.query || data.topic || data.filename || data.source || process, process, 80).toLowerCase();
  const dedupeKey = `${process}:${phase}:${identity}`;
  if (desktop) desktop = { ...desktop, operationId, dedupeKey };
  return { process, phase, operationId, speech, notification: desktop, priority, dedupeKey };
}

export function createAnnouncementDeduper(ttlMs = DEFAULT_TTL_MS) {
  const recent = new Map();
  return {
    accept(key, now = Date.now()) {
      if (!key) return true;
      const previous = recent.get(key);
      if (previous != null && now - previous < ttlMs) return false;
      recent.set(key, now);
      for (const [storedKey, timestamp] of recent) {
        if (now - timestamp >= ttlMs) recent.delete(storedKey);
      }
      return true;
    },
    clear() {
      recent.clear();
    },
  };
}
