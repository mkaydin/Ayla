import test from 'node:test';
import assert from 'node:assert/strict';

import {
  buildProcessAnnouncement,
  cleanAnnouncementLabel,
  createAnnouncementDeduper,
} from '../src/lib/process-announcements.js';

test('search announcements use the topic and create a routed completion notification', () => {
  const started = buildProcessAnnouncement('start', 'search', {
    query: 'reinforcement learning',
    operationId: 'search-1',
  });
  assert.equal(started.speech, 'Searching for reinforcement learning papers.');
  assert.equal(started.notification, null);

  const completed = buildProcessAnnouncement('complete', 'search', {
    query: 'reinforcement learning',
    operationId: 'search-1',
    papers: 12,
    themes: 4,
  });
  assert.equal(completed.speech, 'reinforcement learning search complete.');
  assert.equal(completed.notification.target, 'search');
  assert.match(completed.notification.body, /12 papers, 4 themes/);
});

test('document announcements distinguish Quick Extract and OCR', () => {
  assert.equal(
    buildProcessAnnouncement('start', 'doc-process', { method: 'Quick Extract' }).speech,
    'Quick extraction started.',
  );
  assert.equal(
    buildProcessAnnouncement('complete', 'doc-process', { method: 'OCR', chars: 42 }).speech,
    'OCR process finished.',
  );
});

test('graph completion announcement contains refinement counts', () => {
  const announcement = buildProcessAnnouncement('complete', 'graph-analysis', {
    operationId: 'graph-1',
    mergedNodes: 3,
    renamedNodes: 4,
    updatedMeanings: 5,
    updatedNodeTypes: 2,
    relabeledRelations: 6,
  });
  assert.equal(announcement.speech, 'Graph analysis finished.');
  assert.match(announcement.notification.body, /3 merged, 4 renamed, 5 meanings improved, 2 types corrected, 6 relations improved/);
});

test('knowledge ingestion announces topics and documents differently', () => {
  const topic = buildProcessAnnouncement('start', 'kb-ingest', {
    kind: 'topic',
    topic: 'reinforcement learning',
    operationId: 'search-1',
  });
  assert.equal(topic.speech, 'Adding reinforcement learning to knowledge base.');

  const document = buildProcessAnnouncement('complete', 'kb-ingest', {
    kind: 'document',
    source: 'paper.pdf',
    target: 'search',
    operationId: 'paper-1',
  });
  assert.equal(document.speech, 'Document added to knowledge.');
  assert.equal(document.notification.target, 'search');
  assert.match(document.notification.body, /paper\.pdf was added/);
});

test('announcement labels are bounded and stripped of Markdown/control syntax', () => {
  assert.equal(cleanAnnouncementLabel('**Graph**\n`topic` | $x$', 'Topic'), 'Graph topic x');
  assert.equal(cleanAnnouncementLabel('', 'Topic'), 'Topic');
  assert.equal(cleanAnnouncementLabel('x'.repeat(200), 'Topic', 10).length, 10);
});

test('announcement deduper rejects repeats only inside its TTL', () => {
  const deduper = createAnnouncementDeduper(100);
  assert.equal(deduper.accept('search:start:1', 1_000), true);
  assert.equal(deduper.accept('search:start:1', 1_050), false);
  assert.equal(deduper.accept('search:start:1', 1_101), true);
});
