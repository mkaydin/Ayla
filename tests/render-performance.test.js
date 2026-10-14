import test from 'node:test';
import assert from 'node:assert/strict';
import { Ticker } from '@pixi/ticker';

import {
  framebufferPixels,
  normalizeRenderQuality,
  selectRenderResolution,
  tickerFpsLimit,
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

test('Pixi renders smoothly on a 180 Hz clock without fractional limiter jitter', () => {
  const ticker = new Ticker();
  ticker.maxFPS = tickerFpsLimit(60);
  const times = [];
  let now = 0;
  ticker.add(() => times.push(now));
  for (let refresh = 1; refresh <= 1800; refresh++) {
    now = refresh * 1000 / 180;
    ticker.update(now);
  }
  const intervals = times.slice(1).map((time, i) => time - times[i]).sort((a, b) => a - b);
  assert.ok(times.length >= 590 && times.length <= 640);
  assert.ok(intervals[Math.floor(intervals.length * 0.95)] <= 20);
  ticker.destroy();
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
