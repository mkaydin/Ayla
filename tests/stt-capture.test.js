import assert from 'node:assert/strict';
import test from 'node:test';
import {
  appendDeferredPcm,
  floatToPcm16,
  MAX_DEFERRED_PCM_BYTES,
  measureLevel,
  resampleLinear,
} from '../src/stt-capture.js';

test('STT renderer resampler produces the requested sample count', () => {
  const source = new Float32Array(4_800);
  for (let index = 0; index < source.length; index++) {
    source[index] = Math.sin(index / 20);
  }
  const result = resampleLinear(source, 48_000, 16_000);
  assert.equal(result.length, 1_600);
  assert.ok(result.every(Number.isFinite));
});

test('STT renderer PCM encoder clips values and writes little endian', () => {
  const pcm = floatToPcm16(new Float32Array([-2, 0, 2]));
  assert.deepEqual([...pcm], [0, 128, 0, 0, 255, 127]);
});

test('STT renderer resampler validates inputs', () => {
  assert.throws(() => resampleLinear([], 48_000), TypeError);
  assert.throws(() => resampleLinear(new Float32Array(1), 0), RangeError);
});

test('deferred Chat STT keeps PCM locally until the bounded recording is submitted', () => {
  const chunks = [];
  const first = new Uint8Array([1, 2, 3, 4]);
  assert.equal(appendDeferredPcm(chunks, 0, first, 6), 4);
  assert.deepEqual(chunks, [first]);
  assert.throws(
    () => appendDeferredPcm(chunks, 4, new Uint8Array(3), 6),
    /60 second limit/,
  );
  assert.equal(MAX_DEFERRED_PCM_BYTES, 16_000 * 2 * 60);
});

test('local microphone level is available before deferred STT processing', () => {
  const level = measureLevel(new Float32Array([0.5, -0.5, 0, 0]));
  assert.equal(level.peak, 0.5);
  assert.ok(Math.abs(level.rms - Math.sqrt(0.125)) < 1e-6);
});
