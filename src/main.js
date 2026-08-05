// ── ALICE — Main Bootstrap ─────────────────────────────────

import { Application, ShaderSystem, Ticker } from 'pixi.js';
import { install as installCspSafeShaders } from '@pixi/unsafe-eval';
import { Live2DModel } from 'pixi-live2d-display/cubism4';
import 'katex/dist/katex.min.css';
import 'highlight.js/styles/github-dark.css';
import { TtsPlayer } from './tts-player.js';
import {
  loadMk6Model,
  MK6_PARAMETER_IDS,
} from './live2d/mk6-profile.js';
import {
  Mk6ExpressionController,
  Mk6MotionController,
} from './live2d/mk6-controller.js';
import { Mk6AnimationCoordinator } from './live2d/animation-coordinator.js';
import { Mk6ProceduralAnimator } from './live2d/mk6-procedural.js';
import { events } from './lib/events.js';
import {
  getRuntimeInfo,
  hasDesktopBackend,
  invoke,
  listenEvent,
  showDesktopNotification,
} from './lib/ipc.js';
import {
  appendLog,
  getServiceHealthSnapshot,
  initStatusPanel,
} from './ui/status-panel.js';
import { initSidebar, updateSidebarState } from './ui/sidebar.js';
import { getPanel, restoreAllPanelStates } from './ui/panel-system.js';
import {
  framebufferPixels,
  normalizeRenderQuality,
  selectRenderResolution,
} from './lib/render-performance.js';

// Replace Pixi's `new Function` shader uniform generator with its CSP-safe
// implementation. This keeps Electron's production policy free of unsafe-eval.
installCspSafeShaders({ ShaderSystem });
Live2DModel.registerTicker(Ticker);

function storedToggle(key, fallback = true) {
  const value = localStorage.getItem(key);
  return value == null ? fallback : value !== 'false';
}

// ── Global state ───────────────────────────────────────────
const state = {
  autoBlink: true,
  mouseLook: true,
  ttsEnabled: true,
  processAnnouncementsEnabled: storedToggle('alice_process_announcements'),
  desktopNotificationsEnabled: storedToggle('alice_desktop_notifications'),
  nextBlinkAt: performance.now() + 2000 + Math.random() * 3000,
  blinkPhase: 0, blinkStart: 0,
  micLevel: 0, mouthOpen: 0, ttsMouth: 0,
  headX: 0, headY: 0, eyeX: 0, eyeY: 0,
  renderQuality: normalizeRenderQuality(localStorage.getItem('alice_render_quality')),
  renderResolution: 1,
  renderFps: 0,
  isResizing: false,
};

window.__app = state;

// ── Boot Pixi ──────────────────────────────────────────────
const app = new Application({
  view: document.getElementById('canvas'),
  width: window.innerWidth,
  height: window.innerHeight,
  backgroundAlpha: 1,
  backgroundColor: 0x07111e,
  useContextAlpha: false,
  antialias: false,
  autoStart: true,
  // Live2D auto-update uses Ticker.shared. Sharing it with the renderer avoids
  // running two independent requestAnimationFrame loops.
  sharedTicker: true,
  resolution: selectRenderResolution({
    width: window.innerWidth,
    height: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio,
    quality: state.renderQuality,
  }),
  autoDensity: true,
  forceCanvas: false,
  preferWebGLVersion: 2,
  powerPreference: 'high-performance',
  preserveDrawingBuffer: false,
});
app.ticker.maxFPS = 60;
app.ticker.minFPS = 20;
Ticker.shared.maxFPS = 60;
Ticker.shared.minFPS = 20;
state.renderResolution = app.renderer.resolution;

let applyModelView = () => {};
let rendererResizeFrame = 0;
let rendererResizeEnd = 0;
let lastRendererResizeAt = 0;

function resizeRenderer(timestamp = performance.now()) {
  rendererResizeFrame = 0;
  if (state.isResizing && timestamp - lastRendererResizeAt < 32) {
    rendererResizeFrame = requestAnimationFrame(resizeRenderer);
    return;
  }
  lastRendererResizeAt = timestamp;
  const width = Math.max(1, window.innerWidth);
  const height = Math.max(1, window.innerHeight);
  const resolution = selectRenderResolution({
    width,
    height,
    devicePixelRatio: window.devicePixelRatio,
    quality: state.renderQuality,
    resizing: state.isResizing,
  });
  if (Math.abs(app.renderer.resolution - resolution) > 0.001) {
    app.renderer.resolution = resolution;
  }
  app.renderer.resize(width, height);
  state.renderResolution = resolution;
  applyModelView();
}

function scheduleRendererResize() {
  if (!rendererResizeFrame) {
    rendererResizeFrame = requestAnimationFrame(resizeRenderer);
  }
}

window.addEventListener('resize', () => {
  state.isResizing = true;
  app.ticker.maxFPS = 30;
  Ticker.shared.maxFPS = 30;
  scheduleRendererResize();
  clearTimeout(rendererResizeEnd);
  rendererResizeEnd = setTimeout(() => {
    state.isResizing = false;
    app.ticker.maxFPS = 60;
    Ticker.shared.maxFPS = 60;
    scheduleRendererResize();
  }, 160);
}, { passive: true });

state.setRenderQuality = (quality) => {
  state.renderQuality = normalizeRenderQuality(quality);
  localStorage.setItem('alice_render_quality', state.renderQuality);
  scheduleRendererResize();
  events.emit('render-quality-change', state.renderQuality);
};

document.addEventListener('visibilitychange', () => {
  if (document.hidden) app.stop();
  else app.start();
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
  const runtimeInfo = await getRuntimeInfo();
  if (runtimeInfo.runtime === 'electron') {
    appendLog(
      'info',
      `Chromium runtime: Electron ${runtimeInfo.electron}, Chrome ${runtimeInfo.chrome}; Rust backend ${runtimeInfo.backend?.state || 'unknown'}`,
    );
  } else {
    appendLog('info', `Desktop runtime: ${runtimeInfo.runtime}`);
  }

  await waitForCubismCore();
  appendLog('info', 'Cubism Core loaded');

  const model = await loadMk6Model(Live2DModel);
  app.stage.addChild(model);
  window.__model = model;
  appendLog('success', 'Mk6 model, physics, expressions, and motions loaded');

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
  applyModelView = applyView;
  applyView();

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
  const expressionController = new Mk6ExpressionController(model);
  const motionController = new Mk6MotionController(model);
  const sleepAfterMs = Number(localStorage.getItem('alice_sleep_after_ms')) || 5 * 60_000;
  const animationCoordinator = new Mk6AnimationCoordinator({
    expressionController,
    motionController,
    sleepAfterMs,
  });
  const proceduralAnimator = new Mk6ProceduralAnimator(core);
  const resetAuthoredPoseResidue = () => {
    if (!motionController.isFullBodyMotion()) {
      proceduralAnimator.resetAuthoredPoseResidue();
    }
  };
  model.internalModel.on('beforeMotionUpdate', resetAuthoredPoseResidue);
  model.once('destroy', () => {
    model.internalModel?.off?.('beforeMotionUpdate', resetAuthoredPoseResidue);
  });
  const applyExpression = async (id, clearAfterMs = 6_000) => {
    animationCoordinator.applyChatExpression(id, clearAfterMs);
    return true;
  };
  events.on('motion-play', request => {
    const id = typeof request === 'string' ? request : request?.id;
    if (!id) return;
    motionController.play(id, typeof request === 'object' ? request : {}).catch(error => {
      console.warn('Mk6 motion play failed:', error);
    });
  });
  events.on('motion-stop', () => motionController.stop());
  events.on('expression-select', id => {
    animationCoordinator.setManualLayer(id);
  });
  events.on('expression-toggle', id => {
    animationCoordinator.toggleManualLayer(id);
  });
  events.on('expression-clear-all', () => {
    animationCoordinator.clearManualLayers();
  });
  expressionController.onChange(active => {
    events.emit('expression-state', active);
  });
  window.__mk6 = Object.freeze({
    motions: () => motionController.list(),
    expressions: () => expressionController.list(),
    playMotion: (id, options) => motionController.play(id, options),
    stopMotion: () => motionController.stop(),
    applyExpression: id => animationCoordinator.setManualLayer(id),
    toggleExpression: id => animationCoordinator.toggleManualLayer(id),
    clearExpression: () => animationCoordinator.clearManualLayers(),
    clearExpressions: () => animationCoordinator.clearManualLayers(),
    activeExpressions: () => expressionController.active(),
    current: () => ({
      motion: motionController.current(),
      expression: expressionController.current(),
      animationState: animationCoordinator.getState(),
    }),
    animationState: () => animationCoordinator.getState(),
    setSleepAfterMs: milliseconds => {
      animationCoordinator.sleepAfterMs = Math.max(60_000, Number(milliseconds) || 300_000);
      localStorage.setItem('alice_sleep_after_ms', String(animationCoordinator.sleepAfterMs));
    },
  });

  for (const [id, online] of getServiceHealthSnapshot()) {
    animationCoordinator.setServiceHealth(id, online);
  }
  events.on('service-health', ({ id, online }) => {
    animationCoordinator.setServiceHealth(id, online);
  });
  events.on('process-notify', ({ phase, process, data }) => {
    animationCoordinator.handleProcess(phase, process, data);
  });
  events.on('chat-response-state', ({ id = 'chat', active }) => {
    animationCoordinator.setChatResponseActive(id, active);
  });
  animationCoordinator.onChange(({ state: animationState, offline }) => {
    events.emit('animation-state', { state: animationState, offline });
    if (animationState === 'tracking-lost') {
      appendLog('error', `Animation: tracking lost (${offline} offline)`);
    }
  });

  // TTS
  const ttsPlayer = new TtsPlayer(core);
  ttsPlayer.init().catch((e) => console.warn('TTS init failed:', e));
  ttsPlayer.onStateChange(active => {
    animationCoordinator.setSpeechActive('tts-audio', active);
  });

  const ttsPending = [];
  let ttsBusy = false;
  let ttsSequence = 0;
  let activeTtsDedupeKey = '';

  const ttsProcess = async () => {
    if (ttsBusy || ttsPending.length === 0) return;
    ttsBusy = true;
    animationCoordinator.setSpeechActive('tts-generation', true);
    while (ttsPending.length > 0) {
      const request = ttsPending.shift();
      const text = request.text;
      activeTtsDedupeKey = request.dedupeKey;
      try {
        if (hasDesktopBackend()) {
          if (motionController.current() === 'sleep') motionController.stop();
          appendLog('info', 'TTS: ' + text.slice(0, 60) + '...');
          const wav = await invoke('generate_tts', { text });
          if (wav) ttsPlayer.enqueue(wav instanceof Uint8Array ? wav : new Uint8Array(wav));
        }
      } catch (e) {
        console.warn('TTS fail:', e.message || e);
      }
      await new Promise((r) => setTimeout(r, 30));
      activeTtsDedupeKey = '';
    }
    ttsBusy = false;
    animationCoordinator.setSpeechActive('tts-generation', false);
  };

  events.on('tts-schedule', (value) => {
    if (!state.ttsEnabled) return;
    const request = typeof value === 'string' ? { text: value } : (value || {});
    const t = String(request.text || '').trim();
    if (t.length < 2) return;
    const dedupeKey = String(request.dedupeKey || '');
    if (dedupeKey && (
      activeTtsDedupeKey === dedupeKey
      || ttsPending.some(item => item.dedupeKey === dedupeKey)
    )) return;
    ttsPending.push({
      text: t,
      priority: Number.isFinite(request.priority) ? request.priority : 20,
      dedupeKey,
      sequence: ttsSequence++,
    });
    ttsPending.sort((a, b) => b.priority - a.priority || a.sequence - b.sequence);
    ttsProcess();
  });
  events.on('tts-enabled-change', (enabled) => {
    if (enabled) return;
    ttsPending.length = 0;
    ttsPlayer.stop();
    if (hasDesktopBackend()) invoke('stop_tts').catch(() => {});
  });
  events.on('process-announcement', announcement => {
    if (state.processAnnouncementsEnabled && announcement.speech) {
      events.emit('tts-schedule', {
        text: announcement.speech,
        priority: announcement.priority,
        dedupeKey: announcement.dedupeKey,
        source: 'process',
      });
    }
    if (state.desktopNotificationsEnabled && announcement.notification) {
      showDesktopNotification(announcement.notification).catch(error => {
        console.warn('Desktop notification failed:', error?.message || error);
      });
    }
  });

  // Mouse look
  window.addEventListener('mousemove', (e) => {
    animationCoordinator.activity();
    if (!state.mouseLook) return;
    const nx = (e.clientX / window.innerWidth) * 2 - 1;
    const ny = (e.clientY / window.innerHeight) * 2 - 1;
    proceduralAnimator.setPointerNormalized(nx, ny);
  }, { passive: true });
  for (const eventName of ['pointerdown', 'keydown', 'wheel']) {
    window.addEventListener(eventName, () => animationCoordinator.activity(), { passive: true });
  }

  // Load secondary panels after the Live2D scene is ready. This keeps their
  // markdown, graph, and desktop-backend helpers out of the critical startup bundle.
  const [
    { initChatPanel },
    { initOcrPanel },
    { initSearchPanel },
    { initKbPanel },
  ] = await Promise.all([
    import('./ui/chat-panel.js'),
    import('./ui/ocr-panel.js'),
    import('./ui/search-panel.js'),
    import('./ui/kb-panel.js'),
  ]);

  // Init panels
  initChatPanel(applyExpression);
  initOcrPanel();
  initSearchPanel();
  initKbPanel();
  updateSidebarState();
  restoreAllPanelStates();
  await listenEvent('notification-clicked', event => {
    const target = event.payload?.target;
    const panel = getPanel(target) || getPanel('status');
    panel?.show();
    panel?.focus();
    updateSidebarState();
    events.emit('notification-open', {
      target,
      operationId: event.payload?.operationId || '',
    });
  });

  appendLog('success', 'Ready. Type to talk.');

  // Log renderer info
  const renderType = app.renderer.type === 2 ? 'WebGL2' : app.renderer.type === 1 ? 'WebGL1' : 'Canvas2D';
  appendLog(
    'info',
    `Renderer: ${renderType} · ${state.renderQuality} · ${app.renderer.resolution}x (${app.renderer.width}x${app.renderer.height})`,
  );

  // ── Per-frame update ─────────────────────────────────────
  let measuredFrames = 0;
  let measureStartedAt = performance.now();
  app.ticker.add(() => {
    expressionController.update(app.ticker.deltaMS);
    const now = performance.now();
    animationCoordinator.tick(now);
    measuredFrames++;
    if (now - measureStartedAt >= 1000) {
      state.renderFps = Math.round((measuredFrames * 1000) / (now - measureStartedAt));
      events.emit('render-stats', {
        fps: state.renderFps,
        quality: state.renderQuality,
        resolution: state.renderResolution,
        framebufferPixels: framebufferPixels(
          window.innerWidth,
          window.innerHeight,
          state.renderResolution,
        ),
      });
      measuredFrames = 0;
      measureStartedAt = now;
    }

    const authoredPoseActive = motionController.isFullBodyMotion()
      || !animationCoordinator.isProceduralEnabled();
    proceduralAnimator.update(app.ticker.deltaMS, now, {
      enabled: !authoredPoseActive,
      working: animationCoordinator.isWorking(),
    });

    const ttsActive = ttsPlayer.isActive();
    const ttsLevel = ttsActive ? ttsPlayer.getMouthLevel() : 0;

    if (!authoredPoseActive && state.ttsEnabled && ttsActive) {
      state.ttsMouth += (ttsLevel - state.ttsMouth) * 0.35;
      const mouthLevel = Math.min(1, state.ttsMouth * 5.0);
      core.setParameterValueById(MK6_PARAMETER_IDS.mouthOpen, mouthLevel);
      core.setParameterValueById(MK6_PARAMETER_IDS.jawOpen, mouthLevel * 0.72);
    } else if (!authoredPoseActive) {
      state.ttsMouth *= 0.85;
      const idle = !state.ttsEnabled ? 0.03 : 0.04;
      state.mouthOpen += ((Math.sin(now / 700) * idle + idle) - state.mouthOpen) * 0.05;
      core.setParameterValueById(MK6_PARAMETER_IDS.mouthOpen, state.mouthOpen);
      core.setParameterValueById(MK6_PARAMETER_IDS.jawOpen, state.mouthOpen * 0.35);
    }

    if (state.autoBlink && !authoredPoseActive) {
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
      core.setParameterValueById(MK6_PARAMETER_IDS.eyeLeftOpen, eyeOpen, 1);
      core.setParameterValueById(MK6_PARAMETER_IDS.eyeRightOpen, eyeOpen, 1);
    }
  });
}

main().catch((err) => {
  console.error(err);
  appendLog('error', 'ERROR: ' + err.message);
});
