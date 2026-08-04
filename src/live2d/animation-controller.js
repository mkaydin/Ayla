const clamp01 = value => Math.min(1, Math.max(0, value));

function smoothstep(value) {
  const t = clamp01(value);
  return t * t * (3 - 2 * t);
}

function sampleKeyframes(keyframes, progress) {
  if (!keyframes.length) return 0;
  if (progress <= keyframes[0].at) return keyframes[0].value;
  for (let index = 1; index < keyframes.length; index++) {
    const right = keyframes[index];
    if (progress > right.at) continue;
    const left = keyframes[index - 1];
    const span = Math.max(Number.EPSILON, right.at - left.at);
    const amount = smoothstep((progress - left.at) / span);
    return left.value + (right.value - left.value) * amount;
  }
  return keyframes[keyframes.length - 1].value;
}

function sampleAnimation(definition, progress, intensity) {
  const values = new Map();
  for (const track of definition.tracks) {
    values.set(track.parameter, {
      mode: track.mode,
      value: sampleKeyframes(track.keyframes, progress) * intensity,
    });
  }
  return values;
}

export class Live2DAnimationController {
  constructor(core, profile, catalog) {
    this.core = core;
    this.profile = profile;
    this.catalog = catalog;
    this.active = null;
    this.transition = null;
    this.resetParameters = new Set();
  }

  list() {
    return [...this.catalog.values()];
  }

  isPlaying() {
    return Boolean(this.active);
  }

  currentAnimation() {
    return this.active?.definition.id || null;
  }

  play(id, options = {}) {
    const definition = this.catalog.get(id);
    if (!definition) throw new Error(`Unknown Live2D animation '${id}'`);
    const transitionMs = Math.max(0, Number(options.transitionMs ?? 180));
    if (this.active && transitionMs > 0) {
      this.transition = {
        elapsedMs: 0,
        durationMs: transitionMs,
        values: this._sampleActive(),
      };
    } else {
      this.transition = null;
    }
    this.active = {
      definition,
      elapsedMs: 0,
      loop: options.loop ?? definition.loop,
      intensity: Math.max(0, Number(options.intensity ?? 1)),
    };
    for (const item of definition.tracks) this.resetParameters.add(item.parameter);
    return definition;
  }

  stop({ transitionMs = 160 } = {}) {
    if (!this.active) return false;
    if (transitionMs > 0) {
      this.transition = {
        elapsedMs: 0,
        durationMs: transitionMs,
        values: this._sampleActive(),
        fadeOut: true,
      };
    }
    this.active = null;
    return true;
  }

  _sampleActive() {
    if (!this.active) return new Map();
    const { definition, elapsedMs, loop, intensity } = this.active;
    const duration = Math.max(1, definition.durationMs);
    const progress = loop
      ? (elapsedMs % duration) / duration
      : clamp01(elapsedMs / duration);
    return sampleAnimation(definition, progress, intensity);
  }

  update(deltaMs, baseValues = {}) {
    const elapsed = Math.max(0, Number(deltaMs) || 0);
    let completed = false;
    if (this.active) {
      this.active.elapsedMs += elapsed;
      if (!this.active.loop && this.active.elapsedMs >= this.active.definition.durationMs) {
        this.active.elapsedMs = this.active.definition.durationMs;
        completed = true;
      }
    }

    let values = completed ? new Map() : this._sampleActive();
    if (this.transition) {
      this.transition.elapsedMs += elapsed;
      const mix = smoothstep(this.transition.elapsedMs / this.transition.durationMs);
      const blended = new Map();
      const ids = new Set([...this.transition.values.keys(), ...values.keys()]);
      for (const id of ids) {
        const before = this.transition.values.get(id) || { mode: 'add', value: 0 };
        const after = this.transition.fadeOut
          ? { mode: before.mode, value: 0 }
          : values.get(id) || { mode: before.mode, value: 0 };
        blended.set(id, {
          mode: after.mode,
          value: before.value + (after.value - before.value) * mix,
        });
      }
      values = blended;
      if (mix >= 1) this.transition = null;
    }

    for (const id of this.resetParameters) {
      const base = Number.isFinite(baseValues[id])
        ? baseValues[id]
        : this.profile.defaultValue(id);
      const sample = values.get(id);
      let value = base;
      if (sample?.mode === 'overwrite') value = sample.value;
      else if (sample?.mode === 'multiply') value = base * sample.value;
      else if (sample) value = base + sample.value;
      this.core.setParameterValueById(id, this.profile.clamp(id, value));
    }

    if (
      completed
      && this.active
      && !this.transition
    ) {
      this.active = null;
    }

    if (!this.active && !this.transition) this.resetParameters.clear();
    return values;
  }
}

export { sampleKeyframes };
