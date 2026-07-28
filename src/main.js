// ── ALICE — Main Bootstrap ─────────────────────────────────

import { Application, Ticker } from 'pixi.js';
import { Live2DModel } from 'pixi-live2d-display/cubism4';
import { TtsPlayer } from './tts-player.js';
import { events } from './lib/events.js';
import { initStatusPanel, appendLog } from './ui/status-panel.js';
import { initChatPanel } from './ui/chat-panel.js';
import { initOcrPanel } from './ui/ocr-panel.js';
import { initSearchPanel } from './ui/search-panel.js';
import { initKbPanel } from './ui/kb-panel.js';
import { initSidebar, updateSidebarState } from './ui/sidebar.js';
import { restoreAllPanelStates } from './ui/panel-system.js';

Live2DModel.registerTicker(Ticker);

const MODEL_URL = '/DemonGirl/demongirl.model3.json';
const EXPRESSIONS = ['Amazed', 'Angry', 'Cry', 'Love', 'Nervous', 'Sleepy', 'Customise'];

// ── Global state ───────────────────────────────────────────
const state = {
  autoBlink: true,
  mouseLook: true,
  ttsEnabled: true,
  nextBlinkAt: performance.now() + 2000 + Math.random() * 3000,
  blinkPhase: 0, blinkStart: 0,
  micLevel: 0, mouthOpen: 0, ttsMouth: 0,
  headX: 0, headY: 0, eyeX: 0, eyeY: 0,
};

window.__app = state;

// ── Boot Pixi ──────────────────────────────────────────────
const app = new Application({
  view: document.getElementById('canvas'),
  resizeTo: window,
  backgroundAlpha: 0,
  antialias: true,
  autoStart: true,
  resolution: window.devicePixelRatio || 1,
  autoDensity: true,
  forceCanvas: false,
  preferWebGLVersion: 2,
  powerPreference: 'high-performance',
  preserveDrawingBuffer: false,
});

async function waitForCubismCore(timeoutMs = 10000) {
  const t0 = performance.now();
  while (typeof window.Live2DCubismCore === 'undefined') {
    if (performance.now() - t0 > timeoutMs) throw new Error('Live2D Cubism Core failed to load');
    await new Promise((r) => setTimeout(r, 100));
  }
}

// ── Main ───────────────────────────────────────────────────
async function main() {
  appendLog('info', 'Booting...');

  // Init UI panels
  const statusPanel = initStatusPanel();
  initSidebar();

  await waitForCubismCore();
  appendLog('info', 'Cubism Core loaded');

  const model = await Live2DModel.from(MODEL_URL);
  app.stage.addChild(model);
  window.__model = model;

  // Center & fit
  model.scale.set(1);
  model.anchor.set(0, 0);
  const b = model.getLocalBounds();
  const cx = b.x + b.width / 2;
  const cy = b.y + b.height / 2;
  const view = { userZoom: 1, panX: 0, panY: 0 };

  const applyView = () => {
    const base = Math.min(
      (window.innerWidth * 0.9) / b.width,
      (window.innerHeight * 0.9) / b.height,
    );
    const scale = base * view.userZoom;
    model.scale.set(scale);
    model.x = window.innerWidth / 2 - cx * scale + view.panX;
    model.y = window.innerHeight / 2 - cy * scale + view.panY;
  };
  applyView();
  window.addEventListener('resize', applyView);

  // Zoom/pan
  const canvasEl = document.getElementById('canvas');
  canvasEl.addEventListener('wheel', (e) => {
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
    const newZoom = Math.max(0.1, Math.min(8, view.userZoom * factor));
    const rect = canvasEl.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    const dx = mx - (window.innerWidth / 2 + view.panX);
    const dy = my - (window.innerHeight / 2 + view.panY);
    const k = newZoom / view.userZoom;
    view.panX += dx - dx * k;
    view.panY += dy - dy * k;
    view.userZoom = newZoom;
    applyView();
  }, { passive: false });

  let dragging = false, lastX = 0, lastY = 0;
  canvasEl.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    dragging = true; lastX = e.clientX; lastY = e.clientY;
    canvasEl.setPointerCapture(e.pointerId);
  });
  canvasEl.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    view.panX += e.clientX - lastX;
    view.panY += e.clientY - lastY;
    lastX = e.clientX; lastY = e.clientY;
    applyView();
  });
  canvasEl.addEventListener('pointerup', () => { dragging = false; });
  canvasEl.addEventListener('pointercancel', () => { dragging = false; });

  view.reset = () => { view.userZoom = 1; view.panX = 0; view.panY = 0; applyView(); };
  window.__view = view;

  // Live2D core
  const core = model.internalModel.coreModel;
  const internal = model.internalModel;
  if (internal.eyeBlink) {
    internal.eyeBlink.ids = ['ParamEyeLOpen', 'ParamEyeROpen'];
  }

  // TTS
  const ttsPlayer = new TtsPlayer(core);
  ttsPlayer.init().catch((e) => console.warn('TTS init failed:', e));

  const ttsPending = [];
  let ttsBusy = false;

  const ttsProcess = async () => {
    if (ttsBusy || ttsPending.length === 0) return;
    ttsBusy = true;
    while (ttsPending.length > 0) {
      const text = ttsPending.shift();
      try {
        const ipc = window.__TAURI_INTERNALS__;
        const invokeFn = ipc?.invoke || ipc?.ipc?.invoke || ipc?.core?.invoke;
        if (invokeFn) {
          appendLog('info', 'TTS: ' + text.slice(0, 60) + '...');
          const wav = await invokeFn('generate_tts', { text });
          if (wav) ttsPlayer.enqueue(wav instanceof Uint8Array ? wav : new Uint8Array(wav));
        }
      } catch (e) {
        console.warn('TTS fail:', e.message || e);
      }
      await new Promise((r) => setTimeout(r, 30));
    }
    ttsBusy = false;
  };

  events.on('tts-schedule', (text) => {
    if (!state.ttsEnabled) return;
    const t = text.trim();
    if (t.length < 2) return;
    ttsPending.push(t);
    ttsProcess();
  });

  // Expressions
  const exprData = {};
  let currentExpr = null;

  for (const name of EXPRESSIONS) {
    try {
      const r = await fetch(`/DemonGirl/${name}.exp3.json`);
      const json = await r.json();
      exprData[name] = json.Parameters || [];
    } catch (e) {
      console.warn('Failed to load expression:', name, e);
      exprData[name] = [];
    }
  }

  function unapplyCurrent() {
    if (!currentExpr) return;
    for (const [id, delta] of currentExpr.deltas) {
      core.setParameterValueById(id, core.getParameterValueById(id) - delta);
    }
    currentExpr = null;
  }

  const applyExpression = (name) => {
    if (!name || !exprData[name]) { unapplyCurrent(); return; }
    unapplyCurrent();
    const deltas = new Map();
    for (const p of exprData[name]) {
      const cur = core.getParameterValueById(p.Id);
      const val = p.Blend === 'Add' ? p.Value : 0;
      deltas.set(p.Id, val);
      core.setParameterValueById(p.Id, cur + val);
    }
    currentExpr = { name, deltas };
  };

  // Mouse look
  let targetHeadX = 0, targetHeadY = 0;
  window.addEventListener('mousemove', (e) => {
    if (!state.mouseLook) return;
    const nx = (e.clientX / window.innerWidth) * 2 - 1;
    const ny = (e.clientY / window.innerHeight) * 2 - 1;
    targetHeadX = nx * 30;
    targetHeadY = -ny * 30;
    state.eyeX = nx;
    state.eyeY = -ny;
  });

  // Init panels
  initChatPanel(applyExpression);
  initOcrPanel();
  initSearchPanel();
  initKbPanel();
  updateSidebarState();
  restoreAllPanelStates();

  appendLog('success', 'Ready. Type to talk.');

  // Log renderer info
  const renderType = app.renderer.type === 2 ? 'WebGL2' : app.renderer.type === 1 ? 'WebGL1' : 'Canvas2D';
  appendLog('info', `Renderer: ${renderType} (${app.renderer.width}x${app.renderer.height})`);

  // ── Per-frame update ─────────────────────────────────────
  app.ticker.add((delta) => {
    const now = performance.now();

    state.headX += (targetHeadX - state.headX) * 0.1;
    state.headY += (targetHeadY - state.headY) * 0.1;
    core.setParameterValueById('ParamAngleX', state.headX);
    core.setParameterValueById('ParamAngleY', state.headY);
    core.setParameterValueById('ParamAngleZ', state.headX * -0.2);
    core.setParameterValueById('ParamEyeBallX', state.eyeX);
    core.setParameterValueById('ParamEyeBallY', state.eyeY);

    core.setParameterValueById('ParamBreath', (Math.sin(now / 1000) * 0.5 + 0.5));

    const ttsActive = ttsPlayer.isActive();
    const ttsLevel = ttsActive ? ttsPlayer.getMouthLevel() : 0;

    if (state.ttsEnabled && ttsActive) {
      state.ttsMouth += (ttsLevel - state.ttsMouth) * 0.35;
      core.setParameterValueById('ParamMouthOpenY', Math.min(1, state.ttsMouth * 5.0));
    } else {
      state.ttsMouth *= 0.85;
      const idle = !state.ttsEnabled ? 0.03 : 0.04;
      state.mouthOpen += ((Math.sin(now / 700) * idle + idle) - state.mouthOpen) * 0.05;
      core.setParameterValueById('ParamMouthOpenY', state.mouthOpen);
    }

    if (state.autoBlink) {
      const blinkDur = 280;
      if (state.blinkPhase === 0 && now >= state.nextBlinkAt) {
        state.blinkPhase = 1; state.blinkStart = now;
      }
      let eyeOpen = 1;
      if (state.blinkPhase > 0) {
        const t = (now - state.blinkStart) / blinkDur;
        eyeOpen = 1 - Math.sin(Math.min(1, t) * Math.PI);
        if (t >= 1) {
          state.blinkPhase = 0;
          state.nextBlinkAt = now + 2000 + Math.random() * 3500;
        }
      }
      core.setParameterValueById('ParamEyeLOpen', eyeOpen, 1);
      core.setParameterValueById('ParamEyeROpen', eyeOpen, 1);
    }
  });
}

main().catch((err) => {
  console.error(err);
  appendLog('error', 'ERROR: ' + err.message);
});
