const TERMINAL_PHASES = new Set(['complete', 'error', 'cancel']);
const GLOBAL_REQUIRED_SERVICES = Object.freeze(['backend', 'ollama']);
const PROCESS_SERVICES = Object.freeze({
  search: ['backend', 'ollama', 'searxng', 'lightpanda'],
  'doc-process': ['backend'],
  'kb-ingest': ['backend', 'ollama', 'surrealdb', 'qdrant'],
  'graph-analysis': ['backend', 'ollama', 'surrealdb'],
});

const RESULT_EXPRESSIONS = Object.freeze({
  complete: 'Happy',
  error: 'Confused',
  cancel: 'Tired',
});

const RESTRICTED_CHANNELS = new Set(['emotion', 'modifier', 'indicator', 'prop', 'interaction']);

function processKey(process, data = {}) {
  return `${process}:${data.operationId || data.requestId || 'active'}`;
}

/**
 * Turns semantic application state into one coherent Mk6 pose. Layers are
 * leased by owners, so automatic props temporarily supersede (and later
 * restore) manually selected Status-panel layers.
 */
export class Mk6AnimationCoordinator {
  constructor({
    expressionController,
    motionController,
    now = () => performance.now(),
    sleepAfterMs = 5 * 60_000,
    setTimer = setTimeout,
    clearTimer = clearTimeout,
  }) {
    this.expressions = expressionController;
    this.motions = motionController;
    this.now = now;
    this.sleepAfterMs = Math.max(60_000, Number(sleepAfterMs) || 5 * 60_000);
    // Native browser timers throw "Illegal invocation" when stored on an
    // object and later called as `object.timer(...)`. Wrappers preserve an
    // unbound call while still allowing deterministic timers in tests.
    this.setTimer = (callback, delay) => setTimer(callback, delay);
    this.clearTimer = timer => clearTimer(timer);
    this.lastActivityAt = now();
    this.processes = new Map();
    this.chatResponses = new Set();
    this.speechSources = new Set();
    this.serviceHealth = new Map();
    this.claims = new Map();
    this.renderedLayers = new Set();
    this.timers = new Map();
    this.sequence = 0;
    this.state = 'normal';
    this.motionRevision = 0;
    this.listeners = new Set();
  }

  activity(timestamp = this.now()) {
    this.lastActivityAt = timestamp;
    if (this.state === 'sleep') this._evaluate(timestamp);
  }

  tick(timestamp = this.now()) {
    this._evaluate(timestamp);
  }

  setChatResponseActive(owner, active) {
    if (active) this.chatResponses.add(owner || 'chat');
    else this.chatResponses.delete(owner || 'chat');
    this.activity();
    this._evaluate();
  }

  setSpeechActive(owner, active) {
    if (active) this.speechSources.add(owner || 'tts');
    else this.speechSources.delete(owner || 'tts');
    if (active) this.activity();
    this._evaluate();
  }

  handleProcess(phase, process, data = {}) {
    const key = processKey(process, data);
    if (phase === 'start') {
      this.processes.set(key, { process, data });
      this.leaseLayer(`process:${key}`, 'Loading', { priority: 55 });
      this.activity();
    } else if (TERMINAL_PHASES.has(phase)) {
      if (this.processes.has(key)) {
        this.processes.delete(key);
        this.releaseOwner(`process:${key}`);
      } else {
        // Some backends replace the client operation UUID with their session
        // UUID in the result. A terminal event must still release that task.
        for (const [activeKey, active] of [...this.processes]) {
          if (active.process !== process) continue;
          this.processes.delete(activeKey);
          this.releaseOwner(`process:${activeKey}`);
        }
      }
      let result = RESULT_EXPRESSIONS[phase];
      if (phase === 'complete' && process === 'search' && Number(data.papers) === 0) {
        result = 'Confused';
      }
      this.leaseLayer('process-result', result, {
        priority: 70,
        ttlMs: phase === 'error' ? 4_500 : 3_000,
        replaceOwner: true,
      });
    }
    this._evaluate();
  }

  setServiceHealth(id, status) {
    if (!id) return;
    this.serviceHealth.set(id, status === true || status === 'online' || status === 'ready');
    this._evaluate();
  }

  applyChatExpression(id, ttlMs = 6_000) {
    this.releaseOwner('chat-expression');
    if (id) {
      this.leaseLayer('chat-expression', id, {
        priority: 65,
        ttlMs,
      });
    }
  }

  toggleManualLayer(id) {
    const owner = `manual:${id}`;
    if (this.claims.has(owner)) this.releaseOwner(owner);
    else this.leaseLayer(owner, id, { priority: 10 });
  }

  setManualLayer(id) {
    if (!id) return;
    const definition = this.expressions.list().find(item => item.id === id);
    if (definition?.channel === 'emotion') {
      for (const owner of [...this.claims.keys()]) {
        if (owner.startsWith('manual:')) {
          const claim = this.claims.get(owner);
          const item = this.expressions.list().find(value => value.id === claim.id);
          if (item?.channel === 'emotion') this.releaseOwner(owner, false);
        }
      }
    }
    this.leaseLayer(`manual:${id}`, id, { priority: 10 });
  }

  clearManualLayers() {
    for (const owner of [...this.claims.keys()]) {
      if (owner.startsWith('manual:')) this.releaseOwner(owner, false);
    }
    this._reconcileLayers();
  }

  leaseLayer(owner, id, {
    priority = 20,
    ttlMs = 0,
    replaceOwner = false,
  } = {}) {
    if (!owner || !id) return false;
    if (!this.expressions.list().some(item => item.id === id)) return false;
    if (replaceOwner) this.releaseOwner(owner, false);
    this.claims.set(owner, { owner, id, priority, sequence: ++this.sequence });
    this._clearOwnerTimer(owner);
    if (ttlMs > 0) {
      const timer = this.setTimer(() => {
        this.timers.delete(owner);
        this.releaseOwner(owner);
      }, ttlMs);
      // Node test timers support unref; browser timer handles do not.
      timer?.unref?.();
      this.timers.set(owner, timer);
    }
    this._reconcileLayers();
    return true;
  }

  releaseOwner(owner, reconcile = true) {
    this._clearOwnerTimer(owner);
    const changed = this.claims.delete(owner);
    if (changed && reconcile) this._reconcileLayers();
    return changed;
  }

  getState() {
    return this.state;
  }

  isWorking() {
    return this.state === 'working';
  }

  isProceduralEnabled() {
    return this.state !== 'sleep' && this.state !== 'tracking-lost';
  }

  onChange(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  dispose() {
    for (const timer of this.timers.values()) this.clearTimer(timer);
    this.timers.clear();
    this.listeners.clear();
  }

  _evaluate(timestamp = this.now()) {
    const offline = this._requiredOfflineService();
    const inactive = timestamp - this.lastActivityAt >= this.sleepAfterMs;
    const next = offline
      ? 'tracking-lost'
      : this.processes.size > 0
        ? 'working'
        : this.chatResponses.size > 0 || this.speechSources.size > 0
          ? 'speaking'
          : inactive
            ? 'sleep'
            : 'normal';
    if (next === this.state) return;
    const previous = this.state;
    this.state = next;
    this._syncSystemLayers();
    this._syncMotion();
    for (const listener of this.listeners) listener({ state: next, previous, offline });
  }

  _requiredOfflineService() {
    const required = new Set(GLOBAL_REQUIRED_SERVICES);
    for (const { process } of this.processes.values()) {
      for (const service of PROCESS_SERVICES[process] || []) required.add(service);
    }
    for (const service of required) {
      if (this.serviceHealth.get(service) === false) return service;
    }
    return null;
  }

  _syncSystemLayers() {
    this.releaseOwner('system:tablet', false);
    this.releaseOwner('system:pen', false);
    this.releaseOwner('system:eyemask', false);
    if (this.state === 'working') {
      this.leaseLayer('system:tablet', 'Tablet', { priority: 80 });
      this.leaseLayer('system:pen', 'Pen', { priority: 80 });
    } else if (this.state === 'speaking') {
      this.leaseLayer('system:tablet', 'Tablet', { priority: 75 });
    } else if (this.state === 'sleep') {
      this.leaseLayer('system:eyemask', 'Eyemask', { priority: 100 });
    }
    this._reconcileLayers();
  }

  _syncMotion() {
    const desired = this.state === 'tracking-lost'
      ? 'tracking-lost'
      : this.state === 'sleep'
        ? 'sleep'
        : 'idle';
    if (this.motions.current() === desired) return;
    const revision = ++this.motionRevision;
    this.motions.play(desired, { force: true }).catch(error => {
      if (revision === this.motionRevision) console.warn('Mk6 state motion failed:', error);
    });
  }

  _reconcileLayers() {
    const catalog = new Map(this.expressions.list().map(item => [item.id, item]));
    const restricted = this.state === 'sleep' || this.state === 'tracking-lost';
    const claims = [...this.claims.values()]
      .filter(claim => {
        const definition = catalog.get(claim.id);
        if (!definition) return false;
        if (!restricted) return true;
        if (claim.id === 'Eyemask' && this.state === 'sleep') return true;
        return !RESTRICTED_CHANNELS.has(definition.channel) && definition.slots[0] !== 'face-overlay';
      })
      .sort((a, b) => b.priority - a.priority || b.sequence - a.sequence);

    const selected = new Set();
    const occupied = new Set();
    for (const claim of claims) {
      const definition = catalog.get(claim.id);
      if (selected.has(claim.id)) continue;
      if (definition.slots.some(slot => occupied.has(slot))) continue;
      selected.add(claim.id);
      definition.slots.forEach(slot => occupied.add(slot));
    }

    for (const id of this.renderedLayers) {
      if (!selected.has(id)) this.expressions.deactivate(id, false);
    }
    for (const id of selected) {
      if (!this.renderedLayers.has(id)) this.expressions.activate(id);
    }
    this.renderedLayers = selected;
    // Deactivation with emit=false is grouped into one UI update here.
    this.expressions.notify?.();
  }

  _clearOwnerTimer(owner) {
    const timer = this.timers.get(owner);
    if (timer != null) this.clearTimer(timer);
    this.timers.delete(owner);
  }
}
