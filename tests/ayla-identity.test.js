import test from 'node:test';
import assert from 'node:assert/strict';

import {
  CORE_IDENTITY,
  FACT_EXTRACTION_PROMPT,
  RELATION_EXTRACTION_PROMPT,
  SUMMARIZATION_PROMPT,
} from '../src/lib/prompts.js';

test('runtime identity presents Ayla as a coworker, not the legacy persona', () => {
  assert.match(CORE_IDENTITY, /You are AYLA/);
  assert.match(CORE_IDENTITY, /AI coworker and research partner/);
  assert.match(CORE_IDENTITY, /RESPECT USER AGENCY/);
  assert.doesNotMatch(CORE_IDENTITY, /bound in service|serve your master's will|unconditionally loyal/i);
});

test('memory prompt examples use user and Ayla roles', () => {
  assert.match(FACT_EXTRACTION_PROMPT, /Subject is usually "user"/);
  assert.match(RELATION_EXTRACTION_PROMPT, /\[\["user", "USES"/);
  assert.match(SUMMARIZATION_PROMPT, /User: \{user_msg\}/);
  assert.match(SUMMARIZATION_PROMPT, /Ayla: \{assistant_msg\}/);
});
