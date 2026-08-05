import {
  MK6_EXPRESSION_DATA,
  MK6_EXPRESSIONS,
  MK6_MOTIONS,
} from './mk6-profile.js';

const MOTION_PRIORITY = Object.freeze({ idle: 1, normal: 2, force: 3 });

export class Mk6ExpressionController {
  constructor(
    model,
    catalog = MK6_EXPRESSIONS,
    expressionData = model?.[MK6_EXPRESSION_DATA],
    { fadeInMs = 220, fadeOutMs = 180 } = {},
  ) {
    this.model = model;
    this.catalog = new Map(catalog.map(item => [item.id, item]));
    this.expressionData = expressionData || new Map();
    this.fadeInMs = fadeInMs;
    this.fadeOutMs = fadeOutMs;
    this.states = new Map();
    this.slotOwners = new Map();
    this.listeners = new Set();
    this._beforeModelUpdate = () => this.applyFrame();

    const missing = [...this.catalog.keys()].filter(id => !this.expressionData.has(id));
    if (missing.length) {
      throw new Error(`Mk6 expression parameter data is missing: ${missing.join(', ')}`);
    }
    const internal = model?.internalModel;
    internal?.motionManager?.expressionManager?.resetExpression?.();
    internal?.on?.('beforeModelUpdate', this._beforeModelUpdate);
    model?.once?.('destroy', () => this.dispose());
  }

  list() {
    return [...this.catalog.values()];
  }

  async apply(id) {
    if (!id) return this.clearChannel('emotion');
    return this.activate(id);
  }

  activate(id) {
    const definition = this.catalog.get(id);
    if (!definition) throw new Error(`Unknown Mk6 expression '${id}'`);

    const conflicts = new Set(
      definition.slots
        .map(slot => this.slotOwners.get(slot))
        .filter(owner => owner && owner !== id),
    );
    for (const conflict of conflicts) this.deactivate(conflict, false);

    let state = this.states.get(id);
    if (!state) {
      state = { id, definition, weight: 0, target: 1 };
      this.states.set(id, state);
    } else {
      state.target = 1;
    }
    for (const slot of definition.slots) this.slotOwners.set(slot, id);
    this._emit();
    return true;
  }

  async toggle(id) {
    return this.isActive(id) ? this.deactivate(id) : this.activate(id);
  }

  deactivate(id, emit = true) {
    const state = this.states.get(id);
    if (!state || state.target === 0) return false;
    state.target = 0;
    for (const slot of state.definition.slots) {
      if (this.slotOwners.get(slot) === id) this.slotOwners.delete(slot);
    }
    if (emit) this._emit();
    return true;
  }

  clearChannel(channel) {
    let changed = false;
    for (const state of this.states.values()) {
      if (state.definition.channel === channel && state.target > 0) {
        changed = this.deactivate(state.id, false) || changed;
      }
    }
    if (changed) this._emit();
    return true;
  }

  clear() {
    for (const state of this.states.values()) state.target = 0;
    this.slotOwners.clear();
    this._emit();
    return true;
  }

  current() {
    return this.active().find(item => item.channel === 'emotion')?.id || null;
  }

  active() {
    return this.list()
      .map(definition => this.states.get(definition.id))
      .filter(state => state?.target > 0)
      .map(state => ({
        id: state.id,
        label: state.definition.label,
        channel: state.definition.channel,
        slots: state.definition.slots,
        weight: state.weight,
      }));
  }

  isActive(id) {
    return this.states.get(id)?.target > 0;
  }

  update(deltaMs) {
    const elapsed = Math.max(0, Math.min(100, Number(deltaMs) || 0));
    let weightsChanged = false;
    for (const [id, state] of this.states) {
      const duration = state.target > state.weight ? this.fadeInMs : this.fadeOutMs;
      const step = duration <= 0 ? 1 : elapsed / duration;
      const next = state.target > state.weight
        ? Math.min(state.target, state.weight + step)
        : Math.max(state.target, state.weight - step);
      weightsChanged ||= next !== state.weight;
      state.weight = next;
      if (state.target === 0 && state.weight === 0) this.states.delete(id);
    }
    return weightsChanged;
  }

  applyFrame() {
    const core = this.model?.internalModel?.coreModel;
    if (!core) return;
    for (const state of this.states.values()) {
      if (state.weight <= 0) continue;
      for (const parameter of this.expressionData.get(state.id)) {
        if (parameter.blend === 'multiply') {
          if (core.multiplyParameterValueById) {
            core.multiplyParameterValueById(parameter.id, parameter.value, state.weight);
          } else {
            const current = core.getParameterValueById(parameter.id);
            core.setParameterValueById(
              parameter.id,
              current * (1 + (parameter.value - 1) * state.weight),
            );
          }
        } else if (parameter.blend === 'overwrite') {
          core.setParameterValueById(parameter.id, parameter.value, state.weight);
        } else if (core.addParameterValueById) {
          core.addParameterValueById(parameter.id, parameter.value, state.weight);
        } else {
          const current = core.getParameterValueById(parameter.id);
          core.setParameterValueById(parameter.id, current + parameter.value * state.weight);
        }
      }
    }
  }

  onChange(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  notify() {
    this._emit();
  }

  dispose() {
    this.model?.internalModel?.off?.('beforeModelUpdate', this._beforeModelUpdate);
    this.listeners.clear();
    this.states.clear();
    this.slotOwners.clear();
  }

  _emit() {
    const active = this.active();
    for (const listener of this.listeners) listener(active);
  }
}

export class Mk6MotionController {
  constructor(model, catalog = MK6_MOTIONS) {
    this.model = model;
    this.catalog = new Map(catalog.map(item => [item.id, item]));
    this.currentId = 'idle';
    this.listeners = new Set();
    const manager = model.internalModel?.motionManager;
    manager?.on?.('motionStart', group => {
      const motion = this.list().find(item => item.group === group);
      this.currentId = motion?.id || null;
      this._emit();
    });
    manager?.on?.('motionFinish', () => {
      this.currentId = 'idle';
      this._emit();
    });
  }

  list() {
    return [...this.catalog.values()];
  }

  async play(id, { force = true } = {}) {
    const definition = this.catalog.get(id);
    if (!definition) throw new Error(`Unknown Mk6 motion '${id}'`);
    const priority = id === 'idle'
      ? MOTION_PRIORITY.idle
      : force ? MOTION_PRIORITY.force : MOTION_PRIORITY.normal;
    const started = await this.model.motion(definition.group, definition.index, priority);
    if (started) {
      this.currentId = id;
      this._emit();
    }
    return started;
  }

  stop() {
    this.model.internalModel?.motionManager?.stopAllMotions?.();
    this.currentId = 'idle';
    this._emit();
    return true;
  }

  current() {
    return this.currentId;
  }

  isFullBodyMotion() {
    return this.currentId === 'sleep' || this.currentId === 'tracking-lost';
  }

  onChange(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  _emit() {
    for (const listener of this.listeners) listener(this.currentId);
  }
}
