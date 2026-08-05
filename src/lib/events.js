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
    for (const callback of this._listeners.get(event) || []) {
      try {
        callback(data);
      } catch (error) {
        // Animation, notification, and status observers are ancillary. A UI
        // listener must never turn a completed backend operation into a false
        // Search/OCR/KB failure.
        console.error(`Event listener '${event}' failed:`, error);
      }
    }
  }
}

export const events = new AppEvents();
