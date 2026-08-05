import assert from 'node:assert/strict';
import test from 'node:test';

import { events } from '../src/lib/events.js';

test('event listener failures do not abort later application observers', () => {
  const originalError = console.error;
  console.error = () => {};
  let observed = null;
  const throwing = () => { throw new Error('observer failed'); };
  const receiving = value => { observed = value; };
  const offThrowing = events.on('test-listener-isolation', throwing);
  const offReceiving = events.on('test-listener-isolation', receiving);
  try {
    assert.doesNotThrow(() => events.emit('test-listener-isolation', 17));
    assert.equal(observed, 17);
  } finally {
    offThrowing();
    offReceiving();
    console.error = originalError;
  }
});
