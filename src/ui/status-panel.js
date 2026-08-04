// ── Status Panel ───────────────────────────────────────────

import { createPanel } from './panel-system.js';
import { events } from '../lib/events.js';
import {
  canRestartBackend,
  getBackendStatus,
  listenBackendStatus,
  restartBackend,
  serviceHealthUrl,
} from '../lib/ipc.js';
import { RENDER_QUALITY } from '../lib/render-performance.js';
import {
  LIVE2D_ANIMATIONS,
  LIVE2D_EXPRESSIONS,
} from '../live2d/animation-catalog.js';

const SERVICES = [
  { id: 'backend',   name: 'Rust Backend', url: null, backend: true },
  { id: 'ollama',    name: 'Ollama',      url: serviceHealthUrl('ollama', 'http://localhost:11434/api/tags'), parse: (r) => `${r.models?.length || '?'} models` },
  { id: 'surrealdb', name: 'SurrealDB',   url: serviceHealthUrl('surrealdb', 'http://127.0.0.1:8000/health'),      parse: () => 'http://127.0.0.1:8000' },
  { id: 'qdrant',    name: 'Qdrant',      url: serviceHealthUrl('qdrant', 'http://127.0.0.1:6333'),              parse: () => ':6334 (1024-dim)' },
  { id: 'searxng',   name: 'SearXNG',     url: serviceHealthUrl('searxng', 'http://127.0.0.1:8888/search?q=test&format=json'), parse: () => '8 engines, :8888' },
  { id: 'lightpanda',name: 'Lightpanda',  url: serviceHealthUrl('lightpanda', 'http://127.0.0.1:9222/json/version'),  parse: () => 'CDP :9222' },
  { id: 'tts',       name: 'TTS',         url: null,                                  parse: () => 'Piper amy-low' },
  { id: 'embedding', name: 'Embedding',   url: null,                                  parse: () => 'qwen3-embedding:0.6b' },
];

export function initStatusPanel() {
  const panel = createPanel('status', 'SYSTEM STATUS', {
    x: 8, y: 8, width: 340, height: 420,
    minWidth: 260, minHeight: 200,
    fixed: true, closable: false, className: 'status-panel',
  });

  const body = panel.getContentEl();
  body.innerHTML = `
    <div class="status-services" id="svc-list"></div>
    <div class="status-log" id="svc-log"></div>
    <div class="status-render" id="render-stats">Render: measuring…</div>
    <div class="status-footer" id="svc-footer">Memory: --</div>
    <div class="status-animation" id="animation-controls"></div>
    <div class="status-controls" id="svc-controls"></div>
  `;

  buildServiceRows();
  buildAnimationControls();
  buildControls(panel);
  initializeBackendStatus();

  events.on('status-log', (entry) => {
    appendLog(entry.type, entry.message);
  });
  events.on('render-stats', updateRenderStats);

  // Staggered health checks — one service every 5 seconds
  checkAllServices();
  let checkIdx = 0;
  setInterval(() => {
    checkService(SERVICES[checkIdx % SERVICES.length]);
    checkIdx++;
  }, 5000);

  appendLog('info', 'Status panel initialized');

  return panel;
}

function appendOptions(group, items, type) {
  for (const item of items) {
    const option = document.createElement('option');
    option.value = `${type}:${item.id}`;
    option.textContent = item.label;
    group.appendChild(option);
  }
}

function buildAnimationControls() {
  const container = document.getElementById('animation-controls');
  if (!container) return;

  const label = document.createElement('label');
  label.className = 'status-animation-label';
  label.htmlFor = 'status-animation-select';
  label.textContent = 'Animation';

  const select = document.createElement('select');
  select.id = 'status-animation-select';
  select.className = 'status-animation-select';
  const procedural = document.createElement('optgroup');
  procedural.label = 'Procedural animations';
  appendOptions(procedural, LIVE2D_ANIMATIONS, 'animation');
  const expressions = document.createElement('optgroup');
  expressions.label = 'Existing expressions';
  appendOptions(expressions, LIVE2D_EXPRESSIONS, 'expression');
  select.append(procedural, expressions);

  const loopLabel = document.createElement('label');
  loopLabel.className = 'status-animation-loop';
  const loopToggle = document.createElement('input');
  loopToggle.type = 'checkbox';
  loopToggle.id = 'status-animation-loop';
  loopLabel.append(loopToggle, document.createTextNode('Loop'));

  const play = document.createElement('button');
  play.type = 'button';
  play.className = 'status-ctrl-btn status-animation-play';
  play.textContent = '▶ Play';
  play.addEventListener('click', () => {
    const [type, id] = select.value.split(':', 2);
    if (!id) return;
    if (type === 'expression') {
      events.emit('animation-stop', { transitionMs: 120 });
      events.emit('expression-select', id);
    } else {
      events.emit('expression-select', null);
      events.emit('animation-play', {
        id,
        loop: loopToggle.checked,
        transitionMs: 180,
      });
    }
    appendLog(
      'info',
      `${type === 'expression' ? 'Expression' : 'Animation'}: ${select.selectedOptions[0]?.textContent || id}`,
    );
  });

  const stop = document.createElement('button');
  stop.type = 'button';
  stop.className = 'status-ctrl-btn';
  stop.textContent = '■ Stop';
  stop.addEventListener('click', () => {
    events.emit('animation-stop', { transitionMs: 160 });
    events.emit('expression-select', null);
    appendLog('info', 'Animation stopped');
  });

  const updateSelectionMode = () => {
    const isExpression = select.value.startsWith('expression:');
    loopToggle.disabled = isExpression;
    loopLabel.classList.toggle('disabled', isExpression);
    if (!isExpression) {
      const id = select.value.slice('animation:'.length);
      loopToggle.checked = LIVE2D_ANIMATIONS.find(item => item.id === id)?.loop ?? false;
    }
  };
  select.addEventListener('change', updateSelectionMode);
  updateSelectionMode();

  container.append(label, select, loopLabel, play, stop);
}

let lastBackendState = '';
let backendRetryButton = null;

async function initializeBackendStatus() {
  try {
    updateBackendStatus(await getBackendStatus(), false);
  } catch (error) {
    updateBackendStatus({ state: 'failed', error: error.message }, false);
  }
  listenBackendStatus(status => updateBackendStatus(status, true));
}

function updateBackendStatus(status, logChange) {
  const state = status?.state || 'unavailable';
  const dot = document.getElementById('dot-backend');
  const bar = document.getElementById('bar-backend');
  const info = document.getElementById('info-backend');
  if (!dot || !bar || !info) return;

  const presentation = {
    ready: [
      'green',
      100,
      status.backendVersion === 'tauri'
        ? 'ready · Tauri'
        : status.backendVersion
          ? `ready · v${status.backendVersion}`
          : 'ready',
    ],
    starting: ['yellow', 45, 'starting…'],
    restarting: [
      'yellow',
      35,
      `restarting${status.restartAttempt ? ` · attempt ${status.restartAttempt}` : ''}`,
    ],
    stopping: ['yellow', 20, 'stopping…'],
    stopped: ['gray', 5, 'stopped'],
    failed: ['red', 10, 'failed'],
    unavailable: ['gray', 5, 'unavailable'],
  }[state] || ['gray', 5, state];

  setServiceState(dot, bar, info, ...presentation);
  info.title = status?.error || '';
  if (backendRetryButton) {
    backendRetryButton.disabled = ['starting', 'restarting', 'stopping'].includes(state);
    backendRetryButton.classList.toggle('active', state === 'ready');
  }

  if (logChange && state !== lastBackendState) {
    const type = state === 'ready' ? 'success' : state === 'failed' ? 'error' : 'info';
    const detail = status?.error && state === 'failed' ? `: ${status.error}` : '';
    appendLog(type, `Rust backend ${state}${detail}`);
  }
  lastBackendState = state;
}

function buildServiceRows() {
  const list = document.getElementById('svc-list');
  for (const svc of SERVICES) {
    const row = document.createElement('div');
    row.className = 'status-service-row';
    row.innerHTML = `
      <span class="status-dot gray" id="dot-${svc.id}"></span>
      <span class="status-service-name">${svc.name}</span>
      <span class="status-service-bar"><span class="status-service-bar-fill" id="bar-${svc.id}" style="width:0%"></span></span>
      <span class="status-service-info" id="info-${svc.id}">checking...</span>
    `;
    list.appendChild(row);
  }
}

function buildControls(panel) {
  const ctrls = document.getElementById('svc-controls');
  if (!ctrls) return;

  const ttsBtn = document.createElement('button');
  ttsBtn.className = 'status-ctrl-btn active';
  ttsBtn.textContent = '\uD83D\uDD0A TTS: ON';
  ttsBtn.addEventListener('click', () => {
    const on = !window.__app?.ttsEnabled;
    if (window.__app) window.__app.ttsEnabled = on;
    ttsBtn.textContent = on ? '\uD83D\uDD0A TTS: ON' : '\uD83D\uDD07 TTS: OFF';
    ttsBtn.classList.toggle('active', on);
    events.emit('tts-enabled-change', on);
    events.emit('status-log', { type: 'info', message: on ? 'TTS enabled' : 'TTS muted' });
  });
  ctrls.appendChild(ttsBtn);

  const announcementBtn = document.createElement('button');
  const updateAnnouncementButton = () => {
    const on = window.__app?.processAnnouncementsEnabled !== false;
    announcementBtn.textContent = on ? '🗣 Process voice: ON' : '🗣 Process voice: OFF';
    announcementBtn.classList.toggle('active', on);
  };
  announcementBtn.className = 'status-ctrl-btn';
  updateAnnouncementButton();
  announcementBtn.addEventListener('click', () => {
    const on = !(window.__app?.processAnnouncementsEnabled !== false);
    if (window.__app) window.__app.processAnnouncementsEnabled = on;
    localStorage.setItem('alice_process_announcements', String(on));
    updateAnnouncementButton();
    events.emit('status-log', { type: 'info', message: `Process voice ${on ? 'enabled' : 'disabled'}` });
  });
  ctrls.appendChild(announcementBtn);

  const notificationBtn = document.createElement('button');
  notificationBtn.className = 'status-ctrl-btn';
  const updateNotificationButton = () => {
    const on = window.__app?.desktopNotificationsEnabled !== false;
    notificationBtn.textContent = on ? '🔔 Notifications: ON' : '🔕 Notifications: OFF';
    notificationBtn.classList.toggle('active', on);
  };
  updateNotificationButton();
  notificationBtn.addEventListener('click', () => {
    const on = !(window.__app?.desktopNotificationsEnabled !== false);
    if (window.__app) window.__app.desktopNotificationsEnabled = on;
    localStorage.setItem('alice_desktop_notifications', String(on));
    updateNotificationButton();
    events.emit('status-log', { type: 'info', message: `Desktop notifications ${on ? 'enabled' : 'disabled'}` });
  });
  ctrls.appendChild(notificationBtn);

  const qualityModes = Object.keys(RENDER_QUALITY);
  const qualityBtn = document.createElement('button');
  qualityBtn.className = 'status-ctrl-btn';
  const updateQualityButton = () => {
    const quality = window.__app?.renderQuality || 'auto';
    qualityBtn.textContent = `⚡ Render: ${RENDER_QUALITY[quality]?.label || 'Auto'}`;
    qualityBtn.classList.toggle('active', quality !== 'performance');
  };
  updateQualityButton();
  qualityBtn.addEventListener('click', () => {
    const current = window.__app?.renderQuality || 'auto';
    const next = qualityModes[(qualityModes.indexOf(current) + 1) % qualityModes.length];
    window.__app?.setRenderQuality?.(next);
    updateQualityButton();
    events.emit('status-log', {
      type: 'info',
      message: `Render quality: ${RENDER_QUALITY[next].label}`,
    });
  });
  ctrls.appendChild(qualityBtn);

  if (canRestartBackend()) {
    backendRetryButton = document.createElement('button');
    backendRetryButton.className = 'status-ctrl-btn';
    backendRetryButton.textContent = '↻ Restart backend';
    backendRetryButton.addEventListener('click', async () => {
      backendRetryButton.disabled = true;
      appendLog('info', 'Restarting Rust backend…');
      try {
        updateBackendStatus(await restartBackend(), true);
      } catch (error) {
        updateBackendStatus({ state: 'failed', error: error.message }, true);
      }
    });
    ctrls.appendChild(backendRetryButton);
  }
}

const logLines = [];
const MAX_LOG = 50;

export function appendLog(type, message) {
  const el = document.getElementById('svc-log');
  if (!el) return;

  const time = new Date().toLocaleTimeString('en-GB', { hour12: false });
  const line = document.createElement('div');
  line.className = 'status-log-entry' + (type ? ' ' + type : '');
  line.textContent = `${time}  ${message}`;
  el.appendChild(line);
  el.scrollTop = el.scrollHeight;

  logLines.push({ type, message, el: line });
  if (logLines.length > MAX_LOG) {
    const old = logLines.shift();
    if (old.el) old.el.classList.add('old');
  }
}

async function checkAllServices() {
  for (const svc of SERVICES) {
    checkService(svc);
  }
}

async function checkService(svc) {
  const dot = document.getElementById('dot-' + svc.id);
  const bar = document.getElementById('bar-' + svc.id);
  const info = document.getElementById('info-' + svc.id);
  if (!dot || !bar || !info) return;

  if (svc.backend) {
    try {
      updateBackendStatus(await getBackendStatus(), false);
    } catch (error) {
      updateBackendStatus({ state: 'failed', error: error.message }, false);
    }
    return;
  }

  if (!svc.url) {
    setServiceState(dot, bar, info, 'gray', 100, svc.parse(null));
    return;
  }

  const t0 = performance.now();
  try {
    const r = await fetch(svc.url, { signal: AbortSignal.timeout(5000) });
    const ms = performance.now() - t0;
    if (r.ok) {
      const label = svc.parse(await r.json().catch(() => ({})));
      const color = ms > 2000 ? 'yellow' : 'green';
      setServiceState(dot, bar, info, color, 100, label);
    } else {
      setServiceState(dot, bar, info, 'yellow', 50, `HTTP ${r.status}`);
    }
  } catch (e) {
    setServiceState(dot, bar, info, 'red', 20, 'offline');
  }
}

function setServiceState(dot, bar, info, color, width, text) {
  dot.className = 'status-dot ' + color;
  bar.style.width = width + '%';
  bar.className = 'status-service-bar-fill ' + color;
  info.textContent = text;
}

export function updateMemoryStats(nodes, edges, facts) {
  const footer = document.getElementById('svc-footer');
  if (footer) footer.textContent = `Memory: ${nodes} nodes \u00B7 ${edges} edges \u00B7 ${facts} facts`;
}

function updateRenderStats(stats) {
  const render = document.getElementById('render-stats');
  if (!render) return;
  const megapixels = (stats.framebufferPixels / 1_000_000).toFixed(1);
  render.textContent = `Render: ${stats.fps} FPS · ${stats.resolution}× · ${megapixels} MP`;
}
