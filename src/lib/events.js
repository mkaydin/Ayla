// ── App-wide event bus ─────────────────────────────────────

class AppEvents {
  constructor() {
    this._listeners = new Map();
  }

  on(event, callback) {
    if (!this._listeners.has(event)) this._listeners.set(event, []);
    this._listeners.get(event).push(callback);
    return () => this.off(event, callback);
  }

  off(event, callback) {
    const arr = this._listeners.get(event);
    if (arr) this._listeners.set(event, arr.filter((cb) => cb !== callback));
  }

  emit(event, data) {
    (this._listeners.get(event) || []).forEach((cb) => cb(data));
  }
}

export const events = new AppEvents();
