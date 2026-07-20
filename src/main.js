import { Application, Ticker } from 'pixi.js';
// Use the Cubism 4 entry point so we don't need the Cubism 2 runtime (live2d.min.js).
import { Live2DModel } from 'pixi-live2d-display/cubism4';
import { TtsPlayer } from './tts-player.js';

// pixi-live2d-display needs to know about PIXI's ticker
Live2DModel.registerTicker(Ticker);

const MODEL_URL = '/DemonGirl/demongirl.model3.json';

// 7 expressions from the model's .exp3.json files
const EXPRESSIONS = ['Amazed', 'Angry', 'Cry', 'Love', 'Nervous', 'Sleepy', 'Customise'];

const statusEl = document.getElementById('status');
const micBar = document.getElementById('mic-bar');
const mouthBar = document.getElementById('mouth-bar');
const setStatus = (msg) => { statusEl.textContent = msg; };

// ---------------------------------------------------------------------------
// Boot Pixi
// ---------------------------------------------------------------------------
const app = new Application({
  view: document.getElementById('canvas'),
  resizeTo: window,
  backgroundAlpha: 0,
  antialias: true,
  autoStart: true
});

// ---------------------------------------------------------------------------
// Wait for Cubism Core (loaded via <script> in index.html)
// ---------------------------------------------------------------------------
async function waitForCubismCore(timeoutMs = 10000) {
  const t0 = performance.now();
  while (typeof window.Live2DCubismCore === 'undefined') {
    if (performance.now() - t0 > timeoutMs) {
      throw new Error('Live2D Cubism Core failed to load (check network/CDN).');
    }
    await new Promise((r) => setTimeout(r, 100));
  }
}

// ---------------------------------------------------------------------------
// Load the model
// ---------------------------------------------------------------------------
async function main() {
  setStatus('Waiting for Cubism Core…');
  await waitForCubismCore();

  setStatus('Loading model…');
  const model = await Live2DModel.from(MODEL_URL);
  app.stage.addChild(model);
  window.__model = model;  // debug hook

  // Center & fit ("contain" — whole model visible), with user zoom/pan on top.
  // The moc3 canvas frame is smaller than the actual rendered art (hair/horns
  // overflow), so we fit to the real local bounds, not model.width/height.
  model.scale.set(1);
  model.anchor.set(0, 0);
  const b = model.getLocalBounds();     // {x, y, width, height} of actual art @ scale 1
  const cx = b.x + b.width / 2;
  const cy = b.y + b.height / 2;

  // userZoom multiplies the fit scale; panX/panY shift the center (screen px).
  const view = { userZoom: 1, panX: 0, panY: 0 };

  const apply = () => {
    const base = Math.min(
      (window.innerWidth * 0.9) / b.width,
      (window.innerHeight * 0.9) / b.height
    );
    const scale = base * view.userZoom;
    model.scale.set(scale);
    model.x = window.innerWidth / 2 - cx * scale + view.panX;
    model.y = window.innerHeight / 2 - cy * scale + view.panY;
  };
  apply();
  window.addEventListener('resize', apply);

  // --- Interactive zoom: mouse wheel zooms toward the cursor ---
  const canvas = document.getElementById('canvas');
  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
    const newZoom = Math.max(0.1, Math.min(8, view.userZoom * factor));
    // Keep the point under the cursor stationary:
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    // screen point relative to current center
    const dx = mx - (window.innerWidth / 2 + view.panX);
    const dy = my - (window.innerHeight / 2 + view.panY);
    const k = newZoom / view.userZoom;
    view.panX += dx - dx * k;
    view.panY += dy - dy * k;
    view.userZoom = newZoom;
    apply();
  }, { passive: false });

  // --- Interactive pan: drag with left mouse button on empty space ---
  let dragging = false, lastX = 0, lastY = 0;
  canvas.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    dragging = true; lastX = e.clientX; lastY = e.clientY;
    canvas.setPointerCapture(e.pointerId);
  });
  canvas.addEventListener('pointermove', (e) => {
    if (!dragging) return;
    view.panX += e.clientX - lastX;
    view.panY += e.clientY - lastY;
    lastX = e.clientX; lastY = e.clientY;
    apply();
  });
  const endDrag = (e) => { dragging = false; };
  canvas.addEventListener('pointerup', endDrag);
  canvas.addEventListener('pointercancel', endDrag);

  // Expose for keyboard reset and debugging
  view.reset = () => { view.userZoom = 1; view.panX = 0; view.panY = 0; apply(); };
  window.__view = view;

  // The model3.json ships with EMPTY LipSync and EyeBlink groups, so we wire
  // them manually. If the lib created an eyeBlink controller, hand it our IDs;
  // otherwise the manual blink code below drives the params directly.
  const core = model.internalModel.coreModel;
  const internal = model.internalModel;
  if (internal.eyeBlink) {
    internal.eyeBlink.ids = ['ParamEyeLOpen', 'ParamEyeROpen'];
  }

  // TTS player (only active inside Tauri — falls back silently in browser)
  const ttsPlayer = new TtsPlayer(core);
  ttsPlayer.init().catch(e => console.warn('TTS audio init failed:', e));

  // Sequential TTS queue — prevents concurrent IPC calls mixing up order.
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
          setStatus('🔊 Speaking…');
          const wav = await invokeFn('generate_tts', { text });
          if (wav) {
            ttsPlayer.enqueue(wav instanceof Uint8Array ? wav : new Uint8Array(wav));
          }
        }
      } catch(e) {
        console.warn('TTS fail:', e.message || e);
      }
      // short breather to avoid overloading onnxruntime
      await new Promise(r => setTimeout(r, 30));
    }
    ttsBusy = false;
  };

  const ttsSchedule = (text) => {
    const t = text.trim();
    if (t.length < 2) return;
    ttsPending.push(t);
    ttsProcess();
  };

  /**
   * Extract complete sentences from `text` starting at `cursor`.
   * Returns { sentences: string[], cursor: number (new position) }.
   *
   * Falls back to splitting at ~90 chars when no punctuation is found,
   * so long unpunctuated responses get spoken eventually.
   */
  const extractSentences = (text, cursor) => {
    const tail = text.slice(cursor);
    let newCursor = cursor;
    const sentences = [];

    // 1) Punctuation-delimited sentences
    const re = /[^.!?\n]+[.!?\n]+/g;
    let m;
    while ((m = re.exec(tail)) !== null) {
      sentences.push(m[0].trim());
      newCursor = cursor + m.index + m[0].length;
    }

    // 2) Fallback for long text without punctuation
    const leftover = text.slice(newCursor);
    if (leftover.length > 90) {
      const chunk = leftover.slice(0, 90);
      const lastSpace = chunk.lastIndexOf(' ');
      const splitAt = lastSpace > 30 ? lastSpace : 85;
      sentences.push(leftover.slice(0, splitAt).trim());
      newCursor += splitAt;
    }

    return { sentences, cursor: newCursor };
  };
  setStatus('Ready. Type below to talk ↓');

  // ---------------------------------------------------------------------------
  // Input / control state
  // ---------------------------------------------------------------------------
  const state = {
    autoBlink: true,
    lipSync: false,
    mouseLook: true,
    nextBlinkAt: performance.now() + 2000 + Math.random() * 3000,
    blinkPhase: 0,
    blinkStart: 0,
    micLevel: 0,
    mouthOpen: 0,
    ttsMouth: 0,
    headX: 0, headY: 0,
    eyeX: 0, eyeY: 0
  };

  // ---------------------------------------------------------------------------
  // Mouse look (smoothed)
  // ---------------------------------------------------------------------------
  let targetHeadX = 0, targetHeadY = 0;
  window.addEventListener('mousemove', (e) => {
    if (!state.mouseLook) return;
    const nx = (e.clientX / window.innerWidth) * 2 - 1;   // -1..1
    const ny = (e.clientY / window.innerHeight) * 2 - 1;  // -1..1
    targetHeadX = nx * 30;     // ParamAngleX ~= -30..30
    targetHeadY = -ny * 30;    // invert Y (screen down = head down)
    state.eyeX = nx;
    state.eyeY = -ny;
  });

  // ---------------------------------------------------------------------------
  // Microphone lip-sync (Web Audio analyser → ParamMouthOpenY)
  // ---------------------------------------------------------------------------
  let audioCtx, analyser, micData;
  async function enableLipSync() {
    if (window.__TAURI_INTERNALS__) {
      setStatus('Lip-sync uses Voice button in Tauri.');
      return;
    }
    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      const src = audioCtx.createMediaStreamSource(stream);
      analyser = audioCtx.createAnalyser();
      analyser.fftSize = 512;
      analyser.smoothingTimeConstant = 0.6;
      src.connect(analyser);
      micData = new Uint8Array(analyser.frequencyBinCount);
      state.lipSync = true;
      setStatus('Lip-sync ON (mic active).');
    } catch (err) {
      setStatus('Mic: ' + err.message);
      state.lipSync = false;
    }
  }
  function disableLipSync() {
    state.lipSync = false;
    if (audioCtx) { audioCtx.close(); audioCtx = null; }
    analyser = null;
    setStatus('Lip-sync OFF.');
  }
  function readMic() {
    if (!analyser) return 0;
    analyser.getByteFrequencyData(micData);
    let sum = 0;
    // focus on speech band (~200–3000 Hz): bin = freq / (sampleRate/fftSize)
    const sr = audioCtx.sampleRate;
    const binLow = Math.floor(200 / (sr / analyser.fftSize));
    const binHigh = Math.ceil(3000 / (sr / analyser.fftSize));
    for (let i = binLow; i < binHigh; i++) sum += micData[i];
    const avg = sum / (binHigh - binLow) / 255; // 0..1
    // normalize + noise gate
    const gated = avg < 0.05 ? 0 : (avg - 0.05) / 0.45;
    return Math.min(1, gated * 1.8);
  }

  // ---------------------------------------------------------------------------
  // Expression system
  //
  // The model3.json has no "Expressions" section, so pixi-live2d-display's
  // Cubism4 expressionManager is never created and model.expression() returns
  // false silently. We manually load the .exp3.json files and apply parameters.
  // All expressions use Blend: "Add", so values are deltas.
  // ---------------------------------------------------------------------------
  const exprData = {};               // name -> Parameters[]
  let currentExpr = null;            // { name, deltas: Map<paramId, totalAdd> }

  // Pre-fetch all expression definitions
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

  /** Revert the currently applied expression's additive offsets. */
  function unapplyCurrent() {
    if (!currentExpr) return;
    for (const [id, delta] of currentExpr.deltas) {
      const cur = core.getParameterValueById(id);
      core.setParameterValueById(id, cur - delta);
    }
    currentExpr = null;
  }

  /** Apply an expression by name, or '' / null to reset all. */
  const applyExpression = (name) => {
    if (!name || !exprData[name]) {
      unapplyCurrent();
      setStatus('Expression: none');
      return;
    }
    unapplyCurrent();
    const deltas = new Map();
    for (const p of exprData[name]) {
      const cur = core.getParameterValueById(p.Id);
      const val = p.Blend === 'Add' ? p.Value : 0;  // all use Add
      deltas.set(p.Id, val);
      core.setParameterValueById(p.Id, cur + val);
    }
    currentExpr = { name, deltas };
    setStatus('Expression: ' + name);
  };

  // --- Clickable expression buttons in the HUD ---
  document.getElementById('expr-btns').addEventListener('click', (e) => {
    const btn = e.target.closest('.expr-btn');
    if (!btn) return;
    applyExpression(btn.dataset.expr);
  });

  // ---------------------------------------------------------------------------
  // GUI toggle buttons — no keyboard shortcuts needed
  // ---------------------------------------------------------------------------
  const updateBtnLabels = () => {
    const blinkBtn = document.getElementById('btn-blink');
    const lipBtn = document.getElementById('btn-lipsync');
    const lookBtn = document.getElementById('btn-mouse');
    if (blinkBtn) blinkBtn.textContent = 'Blink: ' + (state.autoBlink ? 'ON' : 'OFF');
    if (lipBtn) lipBtn.textContent = 'Mic: ' + (state.lipSync ? 'ON' : 'OFF');
    if (lookBtn) lookBtn.textContent = 'Look: ' + (state.mouseLook ? 'ON' : 'OFF');
  };

  document.getElementById('btn-blink')?.addEventListener('click', () => {
    state.autoBlink = !state.autoBlink;
    updateBtnLabels();
    setStatus('Auto-blink ' + (state.autoBlink ? 'ON' : 'OFF'));
  });

  document.getElementById('btn-lipsync')?.addEventListener('click', async () => {
    if (state.lipSync) { disableLipSync(); } else { await enableLipSync(); }
    updateBtnLabels();
  });

  document.getElementById('btn-mouse')?.addEventListener('click', () => {
    state.mouseLook = !state.mouseLook;
    if (!state.mouseLook) { targetHeadX = 0; targetHeadY = 0; }
    updateBtnLabels();
    setStatus('Mouse look ' + (state.mouseLook ? 'ON' : 'OFF'));
  });

  document.getElementById('btn-reset')?.addEventListener('click', () => {
    targetHeadX = 0; targetHeadY = 0;
    state.eyeX = 0; state.eyeY = 0;
    applyExpression(null);
    window.__view && window.__view.reset();
    setStatus('Pose + view reset.');
  });

  // Voice input toggle (mic → VAD → STT → auto-send to chat)
  let voiceListening = false;
  const voiceBtn = document.getElementById('btn-voice');
  voiceBtn?.addEventListener('click', async () => {
    try {
      const ipc = window.__TAURI_INTERNALS__;
      const invokeFn = ipc?.invoke || ipc?.ipc?.invoke || ipc?.core?.invoke;
      if (!invokeFn) { setStatus('Voice: Tauri only'); return; }
      if (!voiceListening) {
        await invokeFn('start_listening');
        voiceListening = true;
        voiceBtn.textContent = '🎤 Stop';
        setStatus('Listening…');
      } else {
        await invokeFn('stop_listening');
        voiceListening = false;
        voiceBtn.textContent = '🎤 Voice';
        setStatus('Voice stopped.');
      }
    } catch (e) {
      setStatus('Voice error: ' + (e.message || e));
      voiceListening = false;
    }
  });

  // Hook Tauri events for voice recognition
  (async () => {
    try {
      const { listen } = await import('@tauri-apps/api/event');
      listen('partial-transcript', (e) => {
        setStatus('🎤 ' + e.payload);
      });
      listen('final-transcript', (e) => {
        setStatus('Heard: ' + (e.payload || '').slice(0, 60));
      });
      listen('voice-query', (e) => {
        const text = (e.payload || '').trim();
        if (text.length > 0) {
          chatInput.value = text;
          chatInput.style.height = 'auto';
          chatSendMessage();
        }
      });
      listen('speech-detected', () => {
        setStatus('🔊 Heard you…');
      });
      listen('voice-error', (e) => {
        setStatus('Mic error: ' + e.payload);
        voiceListening = false;
        if (voiceBtn) voiceBtn.textContent = '🎤 Voice';
      });
    } catch (e) {
      // event API not available (browser dev mode)
    }
  })();

  // Initial button labels
  updateBtnLabels();

  // ---------------------------------------------------------------------------
  // Per-frame update
  // ---------------------------------------------------------------------------
  app.ticker.add((delta) => {
    const now = performance.now();
    const dt = delta / 60; // approx in 60fps units

    // Smooth head + eye motion
    state.headX += (targetHeadX - state.headX) * 0.1;
    state.headY += (targetHeadY - state.headY) * 0.1;
    core.setParameterValueById('ParamAngleX', state.headX);
    core.setParameterValueById('ParamAngleY', state.headY);
    // subtle head tilt toward side
    core.setParameterValueById('ParamAngleZ', state.headX * -0.2);
    core.setParameterValueById('ParamEyeBallX', state.eyeX);
    core.setParameterValueById('ParamEyeBallY', state.eyeY);

    // Idle breathing (in case the model's auto-breath is off)
    const breath = (Math.sin(now / 1000) * 0.5 + 0.5);
    core.setParameterValueById('ParamBreath', breath);

    // Mouth: TTS audio drives it when active, otherwise idle / mic lip-sync.
    const ttsActive = ttsPlayer.isActive();
    const ttsLevel = ttsActive ? ttsPlayer.getMouthLevel() : 0;

    if (ttsActive) {
      // Smooth the TTS level to avoid jitter
      state.ttsMouth += (ttsLevel - state.ttsMouth) * 0.35;
      core.setParameterValueById('ParamMouthOpenY', Math.min(1, state.ttsMouth * 5.0));
    } else {
      // Falloff: mouth drifts back to idle when TTS stops
      state.ttsMouth *= 0.85;
      if (state.lipSync) {
        const raw = readMic();
        state.micLevel += (raw - state.micLevel) * 0.4;
        state.mouthOpen += (raw - state.mouthOpen) * 0.5;
      } else {
        state.micLevel = 0;
        state.mouthOpen += ((Math.sin(now / 700) * 0.04 + 0.04) - state.mouthOpen) * 0.05;
      }
      core.setParameterValueById('ParamMouthOpenY', state.mouthOpen);
    }

    // Procedural blink (the model's auto-blink is also on, but we keep manual
    // control so the empty EyeBlink group is respected).
    if (state.autoBlink) {
      const blinkDur = 280; // ms total
      if (state.blinkPhase === 0 && now >= state.nextBlinkAt) {
        state.blinkPhase = 1; state.blinkStart = now;
      }
      let eyeOpen = 1;
      if (state.blinkPhase > 0) {
        const t = (now - state.blinkStart) / blinkDur; // 0..1
        eyeOpen = 1 - Math.sin(Math.min(1, t) * Math.PI);
        if (t >= 1) {
          state.blinkPhase = 0;
          state.nextBlinkAt = now + 2000 + Math.random() * 3500;
        }
      }
      // eye-blink auto-update by the lib targets these IDs; but if it doesn't,
      // we also push values directly (clamped low so we don't fight the lib).
      core.setParameterValueById('ParamEyeLOpen', eyeOpen * 1, 1);
      core.setParameterValueById('ParamEyeROpen', eyeOpen * 1, 1);
    }

    // HUD meters
    micBar.style.width = Math.round(state.micLevel * 100) + '%';
    mouthBar.style.width = Math.round(state.mouthOpen * 100) + '%';
  });

  // ---------------------------------------------------------------------------
  // Chat — Ollama gemma4:e4b
  // ---------------------------------------------------------------------------
  const chatMessages = document.getElementById('chat-messages');
  const chatInput = document.getElementById('chat-input');
  const chatSend = document.getElementById('chat-send');
  const chatBox = document.getElementById('chatbox');
  const chatClose = document.getElementById('chat-close');
  const chatOpen = document.getElementById('chat-open');

  const CHAT_SYSTEM = `Your name is Alice. You are a demon girl assistant. Answer questions correctly and helpfully — give real, accurate answers. Add a hint of your mischievous demon-girl personality, but keep it subdued.

You MUST start EVERY answer with an expression tag in brackets that matches your emotional reaction. Choose ONE:
[amazed] — surprising, impressive, or flattering message
[angry] — insulted, rude, or challenged
[cry] — sad or disappointed
[love] — kind, complimentary, or affectionate
[nervous] — shy, embarrassed, or anxious
[sleepy] — boring, repetitive, you feel lazy
[customise] — confused, thinking hard, complex emotion
[neutral] — normal conversation, simple question

Examples:
"You are beautiful!" → [love] Thank you so much master.
"What is 2+2?" → [neutral] 2+2 equals 4.
"You're stupid!" → [angry] How rude, master. I'm trying my best.
"I feel sad today" → [cry] Oh no, master... I hope things get better.

Only output the tag and your response — never explain the tag.`;

  const chatHistory = [{ role: 'system', content: CHAT_SYSTEM }];
  let chatStreaming = false;
  const EXPR_TAGS = ['amazed','angry','cry','love','nervous','sleepy','customise','neutral'];

  const chatScroll = () => { chatMessages.scrollTop = chatMessages.scrollHeight; };

  const chatAdd = (role, content) => {
    const d = document.createElement('div');
    d.className = 'msg ' + role;
    d.textContent = content;
    chatMessages.appendChild(d);
    chatScroll();
    return d;
  };

  const toggleChat = () => {
    const hidden = chatBox.classList.toggle('chat-hidden');
    chatOpen.style.display = hidden ? 'flex' : 'none';
  };

  const chatSendMessage = async () => {
    const query = chatInput.value.trim();
    if (!query || chatStreaming) return;
    chatInput.value = '';
    chatInput.style.height = 'auto';
    chatAdd('user', query);

    ttsPlayer.stop();  // cancel any in-progress speech

    chatHistory.push({ role: 'user', content: query });

    // Build message array from history: system prompt + last 10 exchanges (20 msgs)
    // This avoids duplicating the user message that was already pushed.
    const msgs = [chatHistory[0], ...chatHistory.slice(1).slice(-20)];

    const thinkDiv = chatAdd('model', '⏳');
    chatSend.disabled = true;
    chatStreaming = true;

    try {
      const r = await fetch('/api/ollama/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          model: 'gemma4:e4b', messages: msgs, stream: true,
          options: { temperature: 0.7 }
        })
      });
      if (!r.ok) throw new Error('Ollama: ' + r.status);

      const reader = r.body.getReader();
      const decoder = new TextDecoder();
      let raw = '', buffer = '', tagDone = false, ttsText = '', ttsSent = 0;
      thinkDiv.textContent = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';
        for (const line of lines) {
          if (!line.trim()) continue;
          try {
            const c = JSON.parse(line);
            if (!c.message?.content) continue;
            raw += c.message.content;

            if (!tagDone) {
              const lb = raw.indexOf(']');
              if (raw.startsWith('[') && lb > 0 && lb < 20) {
                const tag = raw.slice(1, lb).toLowerCase();
                if (EXPR_TAGS.includes(tag)) {
                  const name = tag === 'neutral' ? null
                    : tag.charAt(0).toUpperCase() + tag.slice(1);
                  applyExpression(name);
                  const display = raw.slice(lb + 1).replace(/^\s+/, '');
                  thinkDiv.textContent = display;
                  ttsText = display;
                  tagDone = true;
                }
              }
              if (!raw.startsWith('[') && raw.length > 2) {
                tagDone = true;
                thinkDiv.textContent = raw;
                ttsText = raw;
              }
              if (raw.length > 25 && !tagDone) {
                tagDone = true;
                thinkDiv.textContent = raw;
                ttsText = raw;
              }
            } else {
              const chunk = c.message.content;
              thinkDiv.textContent += chunk;
              ttsText += chunk;
            }

            // Sentence-split TTS: use extractSentences() for better edge-case handling
            if (tagDone && ttsText.length > ttsSent) {
              const { sentences, cursor } = extractSentences(ttsText, ttsSent);
              for (const s of sentences) ttsSchedule(s);
              ttsSent = cursor;
            }

            chatScroll();
          } catch (e) {}
        }
      }
      if (buffer.trim()) {
        try {
          const c = JSON.parse(buffer);
          if (c.message?.content) {
            raw += c.message.content;
            if (tagDone) { thinkDiv.textContent += c.message.content; ttsText += c.message.content; }
          }
        } catch (e) {}
      }

      if (!tagDone) {
        const m = raw.match(/^\[(amazed|angry|cry|love|nervous|sleepy|customise|neutral)\]\s*/i);
        if (m) {
          const tag = m[1].toLowerCase();
          const name = tag === 'neutral' ? null
            : tag.charAt(0).toUpperCase() + tag.slice(1);
          applyExpression(name);
          const display = raw.slice(m[0].length);
          thinkDiv.textContent = display;
          ttsText = display;
        } else {
          thinkDiv.textContent = raw;
          ttsText = raw;
        }
      }
      if (!thinkDiv.textContent.trim()) thinkDiv.textContent = '(no response — try again)';

      // Send any remaining text as final TTS chunk
      if (ttsText.length > ttsSent) {
        ttsSchedule(ttsText.slice(ttsSent).trim());
      }

      chatHistory.push({ role: 'assistant',
        content: thinkDiv.textContent.trim() || raw.trim() });
    } catch (e) {
      thinkDiv.textContent = 'Error: ' + e.message;
      thinkDiv.style.color = '#f87171';
    } finally {
      chatStreaming = false;
      chatSend.disabled = false;
      chatInput.focus();
    }
  };

  chatInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); chatSendMessage(); }
  });
  chatInput.style.height = 'auto';
  chatInput.addEventListener('input', () => {
    chatInput.style.height = 'auto';
    chatInput.style.height = Math.min(120, chatInput.scrollHeight) + 'px';
  });
  chatSend.addEventListener('click', chatSendMessage);
  chatClose.addEventListener('click', toggleChat);
  chatOpen.addEventListener('click', toggleChat);

  chatBox.classList.remove('chat-hidden');
  chatOpen.style.display = 'none';
}

main().catch((err) => {
  console.error(err);
  setStatus('ERROR: ' + err.message);
});
