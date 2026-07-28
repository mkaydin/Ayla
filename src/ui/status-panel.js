// ── Status Panel ───────────────────────────────────────────

import { createPanel } from './panel-system.js';
import { events } from '../lib/events.js';

const SERVICES = [
  { id: 'ollama',    name: 'Ollama',      url: 'http://localhost:11434/api/tags', parse: (r) => `${r.models?.length || '?'} models` },
  { id: 'surrealdb', name: 'SurrealDB',   url: 'http://127.0.0.1:8000/health',      parse: () => 'http://127.0.0.1:8000' },
  { id: 'qdrant',    name: 'Qdrant',      url: 'http://127.0.0.1:6333',              parse: () => ':6334 (1024-dim)' },
  { id: 'searxng',   name: 'SearXNG',     url: 'http://127.0.0.1:8888/search?q=test&format=json', parse: () => '8 engines, :8888' },
  { id: 'lightpanda',name: 'Lightpanda',  url: 'http://127.0.0.1:9222/json/version',  parse: () => 'CDP :9222' },
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
    <div class="status-footer" id="svc-footer">Memory: --</div>
    <div class="status-controls" id="svc-controls"></div>
  `;

  buildServiceRows();
  buildControls(panel);

  events.on('status-log', (entry) => {
    appendLog(entry.type, entry.message);
  });

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
    events.emit('status-log', { type: 'info', message: on ? 'TTS enabled' : 'TTS muted' });
  });
  ctrls.appendChild(ttsBtn);
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
