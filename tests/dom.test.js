import test from 'node:test';
import assert from 'node:assert/strict';

import { escapeHtml, safeCssToken } from '../src/lib/dom.js';

test('escapeHtml neutralizes markup and attribute delimiters', () => {
  assert.equal(
    escapeHtml('<img src=x onerror="run()">\''),
    '&lt;img src=x onerror=&quot;run()&quot;&gt;&#39;',
  );
});

test('safeCssToken rejects class-name injection characters', () => {
  assert.equal(safeCssToken('Concept" onclick="run()'), 'Conceptonclickrun');
  assert.equal(safeCssToken(''), 'Other');
});
