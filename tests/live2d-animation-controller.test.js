import assert from 'node:assert/strict';
import test from 'node:test';
import { Live2DAnimationController, sampleKeyframes } from '../src/live2d/animation-controller.js';
import { LIVE2D_ANIMATIONS, LIVE2D_EXPRESSIONS } from '../src/live2d/animation-catalog.js';
import { Live2DParameterProfile } from '../src/live2d/parameter-profile.js';

function fakeCore(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getParameterValueById: id => values.get(id) ?? 0,
    getParameterMinimumValueById: () => -10,
    getParameterMaximumValueById: () => 10,
    getParameterDefaultValueById: () => 0,
    setParameterValueById(id, value) { values.set(id, value); },
  };
}

const catalog = new Map([
  ['once', {
    id: 'once', label: 'Once', durationMs: 1000, loop: false,
    tracks: [{
      parameter: 'ParamAngleX', mode: 'add',
      keyframes: [{ at: 0, value: 0 }, { at: 0.5, value: 20 }, { at: 1, value: 4 }],
    }],
  }],
  ['loop', {
    id: 'loop', label: 'Loop', durationMs: 1000, loop: true,
    tracks: [{
      parameter: 'ParamAngleX', mode: 'add',
      keyframes: [{ at: 0, value: 0 }, { at: 0.5, value: 4 }, { at: 1, value: 0 }],
    }],
  }],
]);

test('manual validation catalog exposes procedural animations and existing expressions', () => {
  assert.equal(LIVE2D_ANIMATIONS.length, 10);
  assert.equal(LIVE2D_EXPRESSIONS.length, 7);
  assert.equal(new Set(LIVE2D_ANIMATIONS.map(item => item.id)).size, 10);
  assert.equal(new Set(LIVE2D_EXPRESSIONS.map(item => item.id)).size, 7);
});

test('animation keyframes interpolate smoothly', () => {
  const value = sampleKeyframes([
    { at: 0, value: 0 },
    { at: 1, value: 10 },
  ], 0.5);
  assert.equal(value, 5);
});

test('one-shot animations clamp values and restore the supplied base', () => {
  const core = fakeCore();
  const controller = new Live2DAnimationController(
    core,
    new Live2DParameterProfile(core),
    catalog,
  );
  controller.play('once', { transitionMs: 0 });
  controller.update(500, { ParamAngleX: 3 });
  assert.equal(core.values.get('ParamAngleX'), 10);
  controller.update(500, { ParamAngleX: 3 });
  assert.equal(core.values.get('ParamAngleX'), 3);
  assert.equal(controller.isPlaying(), false);
});

test('looping animations remain active until explicitly stopped', () => {
  const core = fakeCore();
  const controller = new Live2DAnimationController(
    core,
    new Live2DParameterProfile(core),
    catalog,
  );
  controller.play('loop', { transitionMs: 0 });
  controller.update(1500, { ParamAngleX: 1 });
  assert.equal(core.values.get('ParamAngleX'), 5);
  assert.equal(controller.currentAnimation(), 'loop');
  controller.stop({ transitionMs: 0 });
  controller.update(0, { ParamAngleX: 1 });
  assert.equal(core.values.get('ParamAngleX'), 1);
});
