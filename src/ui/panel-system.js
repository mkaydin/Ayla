// ── Panel System ───────────────────────────────────────────

import { makeDraggable, makeResizable } from '../lib/drag-resize.js';

let zCounter = 100;

class Panel {
  constructor(id, title, opts = {}) {
    this.id = id;
    this.title = title;
    this.el = null;
    this.headerEl = null;
    this.bodyEl = null;
    this.isVisible = false;
    this.isMinimized = false;
    this.isMaximized = false;
    this.x = opts.x ?? 100;
    this.y = opts.y ?? 100;
    this.width = opts.width ?? 500;
    this.height = opts.height ?? 400;
    this.minWidth = opts.minWidth ?? 280;
    this.minHeight = opts.minHeight ?? 180;
    this.fixed = opts.fixed ?? false;
    this.closable = opts.closable ?? true;
    this.resizable = opts.resizable ?? true;
    this.zIndex = opts.zIndex ?? ++zCounter;
    this.className = opts.className ?? '';
    this.preMaximize = null;
    this._onClose = opts.onClose ?? null;
    this._dragState = null;
    this._resizeState = null;
  }

  mount(container) {
    this.el = document.createElement('div');
    this.el.className = 'panel' + (this.className ? ' ' + this.className : '');
    this.el.dataset.panelId = this.id;
    this.el.style.left = this.x + 'px';
    this.el.style.top = this.y + 'px';
    this.el.style.width = this.width + 'px';
    this.el.style.height = this.height + 'px';
    this.el.style.zIndex = this.zIndex;

    // Header
    this.headerEl = document.createElement('div');
    this.headerEl.className = 'panel-header';

    const titleEl = document.createElement('span');
    titleEl.className = 'panel-title';
    titleEl.textContent = this.title;
    this.titleEl = titleEl;
    this.headerEl.appendChild(titleEl);

    const actions = document.createElement('div');
    actions.className = 'panel-actions';

    const minBtn = document.createElement('button');
    minBtn.className = 'panel-btn minimize';
    minBtn.textContent = '\u2013'; // en-dash
    minBtn.title = 'Minimize';
    minBtn.addEventListener('click', (e) => { e.stopPropagation(); this.toggleMinimize(); });
    actions.appendChild(minBtn);

    const maxBtn = document.createElement('button');
    maxBtn.className = 'panel-btn maximize';
    maxBtn.textContent = '\u25A1'; // □
    maxBtn.title = 'Maximize';
    maxBtn.addEventListener('click', (e) => { e.stopPropagation(); this.toggleMaximize(); });
    actions.appendChild(maxBtn);

    if (this.closable) {
      const closeBtn = document.createElement('button');
      closeBtn.className = 'panel-btn close';
      closeBtn.textContent = '\u00D7'; // ×
      closeBtn.title = 'Close';
      closeBtn.addEventListener('click', (e) => { e.stopPropagation(); this.close(); });
      actions.appendChild(closeBtn);
    }

    this.headerEl.appendChild(actions);
    this.el.appendChild(this.headerEl);

    // Body
    this.bodyEl = document.createElement('div');
    this.bodyEl.className = 'panel-body';
    this.el.appendChild(this.bodyEl);

    // Resize handle
    if (this.resizable) {
      const resize = document.createElement('div');
      resize.className = 'resize-handle';
      this.el.appendChild(resize);
      makeResizable(this, resize);
    }

    container.appendChild(this.el);

    // Drag (unless fixed)
    if (!this.fixed) {
      makeDraggable(this, this.headerEl);
    }

    // Click to focus
    this.el.addEventListener('pointerdown', () => this.focus());

    this.show();
  }

  setTitle(title) {
    this.title = title;
    if (this.titleEl) this.titleEl.textContent = title;
  }

  setContent(html) {
    if (this.bodyEl) this.bodyEl.innerHTML = html;
  }

  getContentEl() {
    return this.bodyEl;
  }

  show() {
    if (!this.el) return;
    this.el.style.display = '';
    this.isVisible = true;
    this.focus();
  }

  hide() {
    if (!this.el) return;
    this.el.style.display = 'none';
    this.isVisible = false;
  }

  close() {
    this.hide();
    if (this._onClose) this._onClose(this);
  }

  toggleMinimize() {
    if (this.isMaximized) this.restoreMaximize();
    this.isMinimized = !this.isMinimized;
    this.el.classList.toggle('minimized', this.isMinimized);
  }

  toggleMaximize() {
    if (this.isMinimized) {
      this.isMinimized = false;
      this.el.classList.remove('minimized');
    }
    if (this.isMaximized) {
      this.restoreMaximize();
    } else {
      this.preMaximize = { x: this.x, y: this.y, w: this.width, h: this.height };
      this.el.classList.add('maximized');
      this.isMaximized = true;
    }
  }

  restoreMaximize() {
    if (this.preMaximize) {
      this.x = this.preMaximize.x;
      this.y = this.preMaximize.y;
      this.width = this.preMaximize.w;
      this.height = this.preMaximize.h;
      this.applyBounds();
      this.preMaximize = null;
    }
    this.el.classList.remove('maximized');
    this.isMaximized = false;
  }

  moveTo(x, y) {
    this.x = Math.max(-this.width + 40, Math.min(x, window.innerWidth - 40));
    this.y = Math.max(0, Math.min(y, window.innerHeight - 32));
    this.el.style.left = this.x + 'px';
    this.el.style.top = this.y + 'px';
  }

  resizeTo(w, h) {
    this.width = Math.max(this.minWidth, Math.min(w, window.innerWidth - this.x - 20));
    this.height = Math.max(this.minHeight, Math.min(h, window.innerHeight - this.y - 20));
    this.el.style.width = this.width + 'px';
    this.el.style.height = this.height + 'px';
  }

  applyBounds() {
    this.el.style.left = this.x + 'px';
    this.el.style.top = this.y + 'px';
    this.el.style.width = this.width + 'px';
    this.el.style.height = this.height + 'px';
  }

  focus() {
    if (!this.el || this.fixed) return;
    document.querySelectorAll('.panel.active').forEach(el => el.classList.remove('active'));
    this.el.classList.add('active');
    this.zIndex = ++zCounter;
    this.el.style.zIndex = this.zIndex;
  }

  destroy() {
    if (this.el) this.el.remove();
  }

  saveState() {
    return { x: this.x, y: this.y, width: this.width, height: this.height, isMinimized: this.isMinimized, isVisible: this.isVisible };
  }

  restoreState(state) {
    if (!state) return;
    this.x = state.x ?? this.x;
    this.y = state.y ?? this.y;
    this.width = state.width ?? this.width;
    this.height = state.height ?? this.height;
    this.applyBounds();
    if (state.isMinimized) { this.isMinimized = true; this.el.classList.add('minimized'); }
    if (state.isVisible === false) this.hide();
  }
}

// Singleton registry
export const panels = new Map();

export function createPanel(id, title, opts) {
  if (panels.has(id)) {
    const existing = panels.get(id);
    existing.show();
    return existing;
  }
  const panel = new Panel(id, title, opts);
  panel.mount(opts.container || document.body);
  panels.set(id, panel);
  return panel;
}

export function getPanel(id) {
  return panels.get(id);
}

export function removePanel(id) {
  const p = panels.get(id);
  if (p) { p.destroy(); panels.delete(id); }
}

export function saveAllPanelStates() {
  const states = {};
  for (const [id, panel] of panels) {
    states[id] = panel.saveState();
  }
  localStorage.setItem('alice_panel_states', JSON.stringify(states));
}

export function restoreAllPanelStates() {
  try {
    const raw = localStorage.getItem('alice_panel_states');
    if (!raw) return;
    const states = JSON.parse(raw);
    for (const [id, state] of Object.entries(states)) {
      const panel = panels.get(id);
      if (panel) panel.restoreState(state);
    }
  } catch (e) { /* ignore */ }
}

window.addEventListener('beforeunload', saveAllPanelStates);
