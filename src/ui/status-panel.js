// ── Status Panel ───────────────────────────────────────────

import { createPanel } from './panel-system.js';
import { events } from '../lib/events.js';
import {
  canRestartBackend,
  getBackendStatus,
  invoke,
  listenBackendStatus,
  restartBackend,
  serviceHealthUrl,
} from '../lib/ipc.js';
import { RENDER_QUALITY } from '../lib/render-performance.js';
import {
  MK6_EXPRESSIONS,
  MK6_MOTIONS,
} from '../live2d/mk6-profile.js';

const SERVICES = [
  { id: 'backend',   name: 'Rust Backend', url: null, backend: true },
  { id: 'ollama',    name: 'Ollama',      url: serviceHealthUrl('ollama', 'http://localhost:11434/api/tags'), parse: (r) => `${r.models?.length || '?'} models` },
  { id: 'surrealdb', name: 'SurrealDB',   url: serviceHealthUrl('surrealdb', 'http://127.0.0.1:8000/health'),      parse: () => 'http://127.0.0.1:8000' },
  { id: 'qdrant',    name: 'Qdrant',      url: serviceHealthUrl('qdrant', 'http://127.0.0.1:6333'),              parse: () => ':6334 (1024-dim)' },
  { id: 'searxng',   name: 'SearXNG',     url: serviceHealthUrl('searxng', 'http://127.0.0.1:8888/healthz'),                  parse: () => 'healthy · :8888' },
  { id: 'lightpanda',name: 'Lightpanda',  url: serviceHealthUrl('lightpanda', 'http://127.0.0.1:9222/json/version'),  parse: () => 'CDP :9222' },
  { id: 'stt',       name: 'STT',         url: null, capability: 'stt' },
  { id: 'tts',       name: 'TTS',         url: null, capability: 'tts' },
  { id: 'embedding', name: 'Embedding',   url: null,                                  parse: () => 'qwen3-embedding:0.6b' },
];

const serviceHealth = new Map();

export function getServiceHealthSnapshot() {
  return new Map(serviceHealth);
}

function reportServiceHealth(id, online) {
  if (!id || typeof online !== 'boolean') return;
  if (serviceHealth.get(id) === online) return;
  serviceHealth.set(id, online);
  events.emit('service-health', { id, online });
}

export function initStatusPanel() {
  const panel = createPanel('status', 'SYSTEM STATUS', {
    x: 8, y: 8, width: 340, height: 420,
    minWidth: 260, minHeight: 200,
    fixed: true, closable: true, className: 'status-panel',
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
    option.dataset.label = item.label;
    group.appendChild(option);
  }
}

function buildAnimationControls() {
  const container = document.getElementById('animation-controls');
  if (!container) return;

  const label = document.createElement('label');
  label.className = 'status-animation-label';
  label.htmlFor = 'status-animation-select';
  label.textContent = 'Mk6 Motion / Expression';

  const select = document.createElement('select');
  select.id = 'status-animation-select';
  select.className = 'status-animation-select';
  const motions = document.createElement('optgroup');
  motions.label = 'Authored motions';
  appendOptions(motions, MK6_MOTIONS, 'motion');
  select.appendChild(motions);
  const channelLabels = {
    emotion: 'Emotions',
    modifier: 'Modifiers',
    indicator: 'Indicators',
    appearance: 'Appearance',
    prop: 'Props',
    interaction: 'Interactions',
  };
  for (const [channel, channelLabel] of Object.entries(channelLabels)) {
    const items = MK6_EXPRESSIONS.filter(item => item.channel === channel);
    if (!items.length) continue;
    const group = document.createElement('optgroup');
    group.label = channelLabel;
    appendOptions(group, items, 'expression');
    select.appendChild(group);
  }

  const play = document.createElement('button');
  play.type = 'button';
  play.className = 'status-ctrl-btn status-animation-play';
  play.textContent = '▶ Play';
  play.addEventListener('click', () => {
    const [type, id] = select.value.split(':', 2);
    if (!id) return;
    if (type === 'expression') {
      events.emit('expression-toggle', id);
    } else {
      events.emit('motion-play', { id });
    }
    appendLog(
      'info',
      `${type === 'expression' ? 'Expression layer toggled' : 'Animation'}: ${select.selectedOptions[0]?.dataset.label || id}`,
    );
  });

  const stop = document.createElement('button');
  stop.type = 'button';
  stop.className = 'status-ctrl-btn';
  stop.textContent = '■ Clear';
  stop.addEventListener('click', () => {
    events.emit('motion-stop');
    events.emit('expression-clear-all');
    appendLog('info', 'Mk6 motion and all expression layers cleared');
  });

  const activeLayers = document.createElement('div');
  activeLayers.className = 'status-animation-active';
  activeLayers.textContent = 'Active layers: none';

  const activityState = document.createElement('div');
  activityState.className = 'status-animation-active';
  activityState.textContent = 'Behavior: normal';
  events.on('animation-state', value => {
    const detail = value?.offline ? ` · ${value.offline} offline` : '';
    activityState.textContent = `Behavior: ${value?.state || 'normal'}${detail}`;
  });

  const activeIds = new Set();
  const updateActionLabel = () => {
    const [type, id] = select.value.split(':', 2);
    play.textContent = type === 'expression'
      ? activeIds.has(id) ? '− Remove' : '＋ Add'
      : '▶ Play';
  };
  select.addEventListener('change', updateActionLabel);
  events.on('expression-state', active => {
    activeIds.clear();
    for (const item of active || []) activeIds.add(item.id);
    for (const option of select.querySelectorAll('option[value^="expression:"]')) {
      const id = option.value.slice('expression:'.length);
      option.textContent = `${activeIds.has(id) ? '● ' : ''}${option.dataset.label}`;
    }
    activeLayers.textContent = active?.length
      ? `Active layers: ${active.map(item => item.label).join(' · ')}`
      : 'Active layers: none';
    updateActionLabel();
  });

  container.append(label, select, play, stop, activityState, activeLayers);
  updateActionLabel();
}

let lastBackendState = '';
let backendRetryButton = null;
let ttsLanguageSelect = null;
let systemSttButton = null;
let systemSttSynced = false;

function updateSystemSttButton() {
  if (!systemSttButton) return;
  const enabled = window.__app?.systemSttEnabled === true;
  systemSttButton.textContent = enabled ? '🎧 System STT: OPEN' : '🎧 System STT: CLOSED';
  systemSttButton.classList.toggle('active', enabled);
  systemSttButton.setAttribute('aria-pressed', String(enabled));
}

async function syncSystemSttPreference() {
  if (systemSttSynced) return;
  systemSttSynced = true;
  try {
    const stored = localStorage.getItem('ayla_system_stt_enabled');
    if (stored == null) {
      const status = await invoke('get_stt_status');
      if (window.__app) window.__app.systemSttEnabled = status?.systemEnabled === true;
      updateSystemSttButton();
      return;
    }
    await invoke('set_system_stt_enabled', {
      enabled: window.__app?.systemSttEnabled === true,
    });
  } catch (error) {
    systemSttSynced = false;
    appendLog('error', `System STT preference: ${error?.message || error}`);
  }
}

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
      status.backendVersion ? `ready · v${status.backendVersion}` : 'ready',
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
  if (state === 'ready') reportServiceHealth('backend', true);
  else if (['failed', 'stopped', 'unavailable'].includes(state)) {
    reportServiceHealth('backend', false);
  }
  if (state === 'ready') void syncSystemSttPreference();
  else systemSttSynced = false;
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

  systemSttButton = document.createElement('button');
  systemSttButton.className = 'status-ctrl-btn';
  systemSttButton.title = 'Arm future system-command listening. The manual Chat microphone remains available.';
  updateSystemSttButton();
  systemSttButton.addEventListener('click', async () => {
    const enabled = window.__app?.systemSttEnabled !== true;
    if (window.__app) window.__app.systemSttEnabled = enabled;
    localStorage.setItem('ayla_system_stt_enabled', String(enabled));
    updateSystemSttButton();
    try {
      await invoke('set_system_stt_enabled', { enabled });
      systemSttSynced = true;
      events.emit('status-log', {
        type: 'info',
        message: `System STT ${enabled ? 'opened' : 'closed'} · Chat microphone remains available`,
      });
    } catch (error) {
      systemSttSynced = false;
      events.emit('status-log', {
        type: 'error',
        message: `System STT preference saved; backend sync pending: ${error?.message || error}`,
      });
    }
  });
  ctrls.appendChild(systemSttButton);

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

  const languageControl = document.createElement('label');
  languageControl.className = 'status-language-control';
  const languageLabel = document.createElement('span');
  languageLabel.textContent = 'TTS language';
  ttsLanguageSelect = document.createElement('select');
  ttsLanguageSelect.className = 'status-language-select';
  ttsLanguageSelect.setAttribute('aria-label', 'TTS language');
  for (const [value, label] of [['tr', 'Türkçe'], ['en', 'English']]) {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = label;
    ttsLanguageSelect.appendChild(option);
  }
  ttsLanguageSelect.value = window.__app?.ttsLanguage || 'tr';
  ttsLanguageSelect.addEventListener('change', () => {
    const language = ttsLanguageSelect.value === 'en' ? 'en' : 'tr';
    if (window.__app) window.__app.ttsLanguage = language;
    localStorage.setItem('ayla_tts_language', language);
    events.emit('tts-language-change', language);
    events.emit('status-log', {
      type: 'info',
      message: `TTS language: ${language === 'en' ? 'English' : 'Türkçe'}`,
    });
  });
  languageControl.append(languageLabel, ttsLanguageSelect);
  ctrls.appendChild(languageControl);

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
    localStorage.setItem('ayla_process_announcements', String(on));
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
    localStorage.setItem('ayla_desktop_notifications', String(on));
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
      const status = await getBackendStatus();
      updateBackendStatus(status, false);
      if (status.backendAvailable) {
        const capabilities = await invoke('get_runtime_capabilities');
        if (!capabilities.storageReady) {
          setServiceState(dot, bar, info, 'yellow', 65, 'connected · storage unavailable');
          info.title = 'Chat and speech remain available. Knowledge approval requires persistent storage. An idle backend retries startup automatically.';
        }
      }
    } catch (error) {
      updateBackendStatus({ state: 'failed', error: error.message }, false);
    }
    return;
  }

  if (!svc.url) {
    if (svc.capability) {
      try {
        const capabilities = await invoke('get_runtime_capabilities');
        const capability = capabilities?.[svc.capability];
        if (capability?.available) {
          if (svc.capability === 'tts') updateTtsLanguageCapabilities(capability);
          const sttLanguage = svc.capability === 'stt'
            ? `${capability.languageMode || 'auto'} · ${(capability.supportedLanguages || ['tr', 'en']).join('/')}`
            : null;
          const label = [
            capability.engine || capability.model,
            capability.voice || capability.provider,
            svc.capability === 'tts'
              ? window.__app?.ttsLanguage || capability.language
              : sttLanguage || capability.language,
          ]
            .filter(Boolean)
            .join(' · ');
          setServiceState(dot, bar, info, 'green', 100, label || 'available');
          info.title = capability.modelDirectory || '';
          reportServiceHealth(svc.id, true);
        } else {
          setServiceState(dot, bar, info, 'red', 20, 'unavailable');
          info.title = capability?.reason || 'Capability unavailable';
          reportServiceHealth(svc.id, false);
        }
      } catch (error) {
        setServiceState(dot, bar, info, 'red', 20, 'unavailable');
        info.title = error?.message || String(error);
        reportServiceHealth(svc.id, false);
      }
      return;
    }
    setServiceState(dot, bar, info, 'gray', 100, svc.parse?.(null) || 'built in');
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
      reportServiceHealth(svc.id, true);
    } else {
      setServiceState(dot, bar, info, 'yellow', 50, `HTTP ${r.status}`);
      reportServiceHealth(svc.id, false);
    }
  } catch (e) {
    setServiceState(dot, bar, info, 'red', 20, 'offline');
    reportServiceHealth(svc.id, false);
  }
}

function updateTtsLanguageCapabilities(capability) {
  if (!ttsLanguageSelect) return;
  const supported = Array.isArray(capability.supportedLanguages)
    ? capability.supportedLanguages
    : capability.language ? [capability.language] : [];
  for (const option of ttsLanguageSelect.options) {
    option.disabled = supported.length > 0 && !supported.includes(option.value);
  }
  let selected = window.__app?.ttsLanguage || 'tr';
  if (!localStorage.getItem('ayla_tts_language') && supported.includes(capability.language)) {
    selected = capability.language;
    if (window.__app) window.__app.ttsLanguage = selected;
    ttsLanguageSelect.value = selected;
  }
  if (supported.length > 0 && !supported.includes(selected)) {
    const fallback = supported.includes(capability.language)
      ? capability.language
      : supported[0];
    if (window.__app) window.__app.ttsLanguage = fallback;
    ttsLanguageSelect.value = fallback;
  }
  ttsLanguageSelect.title = supported.length
    ? `Available: ${supported.join(', ')}`
    : 'No language information reported by the TTS backend';
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
