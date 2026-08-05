import assert from 'node:assert/strict';
import test from 'node:test';

import {
  augmentMk6Manifest,
  loadMk6Model,
  MK6_EXPRESSION_DATA,
  MK6_EXPRESSIONS,
  MK6_MOTIONS,
  MK6_PARAMETER_IDS,
} from '../src/live2d/mk6-profile.js';
import {
  Mk6ExpressionController,
  Mk6MotionController,
} from '../src/live2d/mk6-controller.js';
import { Mk6AnimationCoordinator } from '../src/live2d/animation-coordinator.js';
import { Mk6ProceduralAnimator } from '../src/live2d/mk6-procedural.js';

function fakeModel() {
  const listeners = new Map();
  const internalListeners = new Map();
  const calls = [];
  const values = new Map();
  const coreModel = {
    addParameterValueById(id, value, weight = 1) {
      values.set(id, (values.get(id) || 0) + value * weight);
    },
    getParameterValueById(id) { return values.get(id) || 0; },
    setParameterValueById(id, value, weight = 1) {
      const current = values.get(id) || 0;
      values.set(id, current + (value - current) * weight);
    },
  };
  const expressionManager = {
    resetExpression() { calls.push(['resetExpression']); },
  };
  const motionManager = {
    expressionManager,
    on(event, callback) { listeners.set(event, callback); },
    stopAllMotions() { calls.push(['stopAllMotions']); },
  };
  return {
    calls,
    listeners,
    internalListeners,
    values,
    internalModel: {
      motionManager,
      coreModel,
      on(event, callback) { internalListeners.set(event, callback); },
      off(event) { internalListeners.delete(event); },
    },
    async motion(group, index, priority) {
      calls.push(['motion', group, index, priority]);
      return true;
    },
  };
}

function expressionData(overrides = {}) {
  return new Map(MK6_EXPRESSIONS.map(definition => [
    definition.id,
    overrides[definition.id] || [],
  ]));
}

test('Mk6 catalog exposes every authored motion and expression', () => {
  assert.equal(MK6_EXPRESSIONS.length, 21);
  assert.equal(MK6_MOTIONS.length, 3);
  assert.equal(new Set(MK6_EXPRESSIONS.map(item => item.id)).size, 21);
  assert.deepEqual(MK6_MOTIONS.map(item => item.id), ['idle', 'tracking-lost', 'sleep']);
  assert.equal(MK6_PARAMETER_IDS.bodyY, 'BodyAngleY');
});

test('Mk6 manifest augmentation registers local authored assets', () => {
  const original = {
    Version: 3,
    FileReferences: {
      Moc: 'Mk6.moc3',
      Textures: ['Mk6.8192/texture_00.png'],
      Physics: 'Mk6.physics3.json',
    },
    Groups: [],
  };
  const manifest = augmentMk6Manifest(original, '/model/Mk6.model3.json');
  assert.equal(manifest.url, '/model/Mk6.model3.json');
  assert.equal(manifest.FileReferences.Expressions.length, 21);
  assert.equal(manifest.FileReferences.Motions.Idle[0].File, 'Motions/idle.motion3.json');
  assert.equal(manifest.FileReferences.Motions.Sleep[0].File, 'Motions/Sleep.motion3.json');
  assert.deepEqual(manifest.Groups.map(group => group.Ids), [[], []]);
  assert.equal(original.FileReferences.Expressions, undefined);
});

test('Mk6 loader passes the completed manifest to the Cubism runtime', async () => {
  let received;
  const runtime = {
    async from(manifest, options) {
      received = { manifest, options };
      return { loaded: true };
    },
  };
  const fetchModel = async path => ({
    ok: true,
    async json() {
      if (path !== '/__ayla_model__/Mk6.model3.json') return { Parameters: [] };
      return {
        Version: 3,
        FileReferences: {
          Moc: 'Mk6.moc3',
          Textures: ['Mk6.8192/texture_00.png'],
        },
      };
    },
  });
  const model = await loadMk6Model(runtime, fetchModel);
  assert.equal(model.loaded, true);
  assert.equal(model[MK6_EXPRESSION_DATA].size, 21);
  assert.deepEqual(
    model[MK6_EXPRESSION_DATA].get('Tablet').find(item => item.id === 'ParamForearmR1ONOFF'),
    { id: 'ParamForearmR1ONOFF', value: 1, blend: 'overwrite' },
  );
  assert.deepEqual(
    model[MK6_EXPRESSION_DATA].get('Pen').find(item => item.id === 'ParamForearmL1ONOFF'),
    { id: 'ParamForearmL1ONOFF', value: 1, blend: 'overwrite' },
  );
  assert.equal(received.manifest.FileReferences.Expressions.length, 0);
  assert.equal(received.options.motionPreload, 'ALL');
});

test('Mk6 loader gives an actionable error when no local mount exists', async () => {
  await assert.rejects(
    () => loadMk6Model({}, async () => ({ ok: false, status: 404 })),
    /Install Mk6_1\.0 locally or set AYLA_LIVE2D_MODEL_DIR/,
  );
});

test('Mk6 expression controller composes independent slots and replaces conflicts', async () => {
  const model = fakeModel();
  const controller = new Mk6ExpressionController(model, MK6_EXPRESSIONS, expressionData());
  assert.equal(await controller.apply('Happy'), true);
  assert.equal(controller.current(), 'Happy');
  await controller.apply('Tablet');
  await controller.apply('Pen');
  assert.deepEqual(controller.active().map(item => item.id), ['Happy', 'Pen', 'Tablet']);
  await controller.apply('Tracking');
  assert.deepEqual(controller.active().map(item => item.id), ['Happy', 'Tracking']);
  await controller.apply('Eyemask');
  await controller.apply('Outfit');
  assert.deepEqual(
    controller.active().map(item => item.id),
    ['Happy', 'Eyemask', 'Outfit', 'Tracking'],
  );
  await controller.apply('JaketOFF');
  assert.equal(controller.isActive('Outfit'), false);
  assert.equal(controller.isActive('Eyemask'), true);
  controller.clear();
  assert.equal(controller.current(), null);
  assert.deepEqual(model.calls.at(-1), ['resetExpression']);
  await assert.rejects(() => controller.apply('UnknownExpression'), /Unknown Mk6 expression/);
});

test('Mk6 expression mixer fades and applies authored additive parameters before model update', async () => {
  const model = fakeModel();
  const data = expressionData({
    Blush: [{ id: 'ExpFaceBlush', value: 1, blend: 'add' }],
  });
  const controller = new Mk6ExpressionController(
    model,
    MK6_EXPRESSIONS,
    data,
    { fadeInMs: 100, fadeOutMs: 100 },
  );
  await controller.apply('Blush');
  controller.update(50);
  model.internalListeners.get('beforeModelUpdate')();
  assert.equal(model.values.get('ExpFaceBlush'), 0.5);
  await controller.toggle('Blush');
  controller.update(50);
  model.values.delete('ExpFaceBlush');
  model.internalListeners.get('beforeModelUpdate')();
  assert.equal(model.values.get('ExpFaceBlush'), undefined);
  assert.equal(controller.isActive('Blush'), false);
});

test('Mk6 motion controller uses native priority and returns to idle', async () => {
  const model = fakeModel();
  const controller = new Mk6MotionController(model);
  assert.equal(await controller.play('sleep'), true);
  assert.equal(controller.current(), 'sleep');
  assert.equal(controller.isFullBodyMotion(), true);
  assert.deepEqual(model.calls.at(-1), ['motion', 'Sleep', 0, 3]);
  controller.stop();
  assert.equal(controller.current(), 'idle');
  assert.deepEqual(model.calls.at(-1), ['stopAllMotions']);
});

test('animation coordinator restores manual props after a working lease ends', () => {
  const model = fakeModel();
  const expressions = new Mk6ExpressionController(model, MK6_EXPRESSIONS, expressionData());
  const motions = new Mk6MotionController(model);
  const coordinator = new Mk6AnimationCoordinator({
    expressionController: expressions,
    motionController: motions,
    now: () => 0,
  });

  coordinator.toggleManualLayer('Controller');
  assert.equal(expressions.isActive('Controller'), true);
  coordinator.handleProcess('start', 'graph-analysis', { operationId: 'graph-1' });
  assert.equal(coordinator.getState(), 'working');
  assert.equal(expressions.isActive('Controller'), false);
  assert.equal(expressions.isActive('Tablet'), true);
  assert.equal(expressions.isActive('Pen'), true);
  assert.equal(expressions.isActive('Loading'), true);

  coordinator.handleProcess('complete', 'graph-analysis', { operationId: 'graph-1' });
  assert.equal(coordinator.getState(), 'normal');
  assert.equal(expressions.isActive('Controller'), true);
  assert.equal(expressions.isActive('Tablet'), false);
  assert.equal(expressions.isActive('Pen'), false);
  assert.equal(expressions.isActive('Happy'), true);
});

test('animation coordinator gates sleep and tracking-lost poses', () => {
  let now = 0;
  const model = fakeModel();
  const expressions = new Mk6ExpressionController(model, MK6_EXPRESSIONS, expressionData());
  const motions = new Mk6MotionController(model);
  const coordinator = new Mk6AnimationCoordinator({
    expressionController: expressions,
    motionController: motions,
    now: () => now,
    sleepAfterMs: 60_000,
  });

  coordinator.toggleManualLayer('Happy');
  now = 60_001;
  coordinator.tick();
  assert.equal(coordinator.getState(), 'sleep');
  assert.equal(expressions.isActive('Eyemask'), true);
  assert.equal(expressions.isActive('Happy'), false);

  coordinator.activity();
  assert.equal(coordinator.getState(), 'normal');
  assert.equal(expressions.isActive('Happy'), true);
  coordinator.setServiceHealth('backend', false);
  assert.equal(coordinator.getState(), 'tracking-lost');
  assert.equal(expressions.isActive('Happy'), false);
  coordinator.setServiceHealth('backend', true);
  assert.equal(coordinator.getState(), 'normal');
  assert.equal(expressions.isActive('Happy'), true);
});

test('animation coordinator uses tablet for speaking and tablet plus pen for work', () => {
  const model = fakeModel();
  const expressions = new Mk6ExpressionController(model, MK6_EXPRESSIONS, expressionData());
  const coordinator = new Mk6AnimationCoordinator({
    expressionController: expressions,
    motionController: new Mk6MotionController(model),
    now: () => 0,
  });

  coordinator.setChatResponseActive('turn-1', true);
  assert.equal(coordinator.getState(), 'speaking');
  assert.equal(expressions.isActive('Tablet'), true);
  assert.equal(expressions.isActive('Pen'), false);
  coordinator.handleProcess('start', 'search', { operationId: 'search-1' });
  assert.equal(coordinator.getState(), 'working');
  assert.equal(expressions.isActive('Tablet'), true);
  assert.equal(expressions.isActive('Pen'), true);
});

test('animation coordinator invokes injected timers without a receiver', () => {
  let timerReceiver = 'not-called';
  let clearReceiver = 'not-called';
  function timer() {
    timerReceiver = this;
    return 42;
  }
  function clear() {
    clearReceiver = this;
  }
  const model = fakeModel();
  const expressions = new Mk6ExpressionController(model, MK6_EXPRESSIONS, expressionData());
  const coordinator = new Mk6AnimationCoordinator({
    expressionController: expressions,
    motionController: new Mk6MotionController(model),
    now: () => 0,
    setTimer: timer,
    clearTimer: clear,
  });

  coordinator.applyChatExpression('Happy', 1_000);
  coordinator.applyChatExpression(null);
  assert.equal(timerReceiver, undefined);
  assert.equal(clearReceiver, undefined);
});

test('procedural animator drives head, body, legs, and working pen inputs', () => {
  const values = new Map();
  const core = {
    setParameterValueById(id, value) { values.set(id, value); },
  };
  const animator = new Mk6ProceduralAnimator(core);
  animator.setPointerNormalized(0.8, -0.5);
  animator.update(16, 1_500, { enabled: true, working: true });

  assert.notEqual(values.get(MK6_PARAMETER_IDS.headX), 0);
  assert.notEqual(values.get(MK6_PARAMETER_IDS.bodyX), 0);
  assert.notEqual(values.get(MK6_PARAMETER_IDS.rightLegX), undefined);
  assert.notEqual(values.get(MK6_PARAMETER_IDS.leftLegY), undefined);
  assert.notEqual(values.get(MK6_PARAMETER_IDS.mousePositionX), undefined);
  assert.notEqual(values.get(MK6_PARAMETER_IDS.leftArmWork), undefined);

  animator.resetAuthoredPoseResidue();
  assert.equal(values.get('ExpMask'), 0);
  assert.equal(values.get('ExpSleeping'), 0);
  assert.equal(values.get('ParamMouthMode'), 0);
});
