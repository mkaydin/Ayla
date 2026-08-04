import test from 'node:test';
import assert from 'node:assert/strict';

import {
  framebufferPixels,
  normalizeRenderQuality,
  selectRenderResolution,
} from '../src/lib/render-performance.js';

test('adaptive resolution stays inside practical fullscreen pixel budgets', () => {
  const resolution = selectRenderResolution({
    width: 3840,
    height: 2160,
    devicePixelRatio: 2,
    quality: 'auto',
  });
  assert.equal(resolution, 0.625);
  assert.ok(framebufferPixels(3840, 2160, resolution) < 3_300_000);
});

test('live resize lowers resolution and quality modes remain bounded', () => {
  const settled = selectRenderResolution({
    width: 1920,
    height: 1080,
    devicePixelRatio: 2,
    quality: 'auto',
  });
  const resizing = selectRenderResolution({
    width: 1920,
    height: 1080,
    devicePixelRatio: 2,
    quality: 'auto',
    resizing: true,
  });
  assert.ok(resizing < settled);
  assert.equal(normalizeRenderQuality('invalid'), 'auto');
});
