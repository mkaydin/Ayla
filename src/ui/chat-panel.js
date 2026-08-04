// ── Chat Panel ─────────────────────────────────────────────

import { createPanel, getPanel } from './panel-system.js';
import {
  hasDesktopBackend,
  invoke,
  invokeStreamCancelable,
} from '../lib/ipc.js';
import { MarkdownSpeechStream, renderMarkdown } from '../lib/markdown.js';
import { events } from '../lib/events.js';
import { CORE_IDENTITY, EXPR_TAGS, EXPR_MAP } from '../lib/prompts.js';
import { generateAliceResponse } from '../lib/alice-responses.js';

export function initChatPanel(applyExpressionFn) {
  const panel = createPanel('chat', 'Chat', {
    x: Math.round(window.innerWidth * 0.2),
    y: window.innerHeight - 440,
    width: 640, height: 420,
    minWidth: 480, minHeight: 360,
    className: 'chat-panel',
  });

  const body = panel.getContentEl();
  body.innerHTML = `
    <div class="chat-layout">
      <div class="chat-sidebar">
        <div class="chat-sidebar-header">
          <span style="font-size:11px;color:var(--text-muted)">Sessions</span>
          <button class="chat-new-btn" id="chat-new-btn">+ New</button>
        </div>
        <div class="chat-session-list" id="chat-session-list"></div>
        <div class="chat-session-actions">
          <button class="chat-delete-btn" id="chat-delete-btn">Delete session</button>
          <button class="ocr-action-btn approve" id="chat-to-kb-btn" title="Extract knowledge from this conversation">\uD83E\uDDE0 Send to KB</button>
        </div>
      </div>
      <div class="chat-messages-area">
        <div class="chat-messages" id="chat-messages"></div>
        <div class="chat-input-row">
          <textarea class="chat-input" id="chat-input" placeholder="Ask Alice..." rows="1"></textarea>
          <button class="chat-send-btn" id="chat-send-btn">\u2191</button>
        </div>
      </div>
    </div>
  `;

  // Chat state
  const sessions = new Map();
  let activeSession = null;
  let streaming = false;
  let streamOperation = null;
  let streamCancelRequested = false;

  const defaultSession = {
    id: crypto.randomUUID(),
    title: 'New Chat',
    messages: [{ role: 'system', content: CORE_IDENTITY }],
    createdAt: Date.now(),
    updatedAt: Date.now(),
  };
  sessions.set(defaultSession.id, defaultSession);
  activeSession = defaultSession;

  const msgContainer = document.getElementById('chat-messages');
  const inputEl = document.getElementById('chat-input');
  const sendBtn = document.getElementById('chat-send-btn');
  const sessionList = document.getElementById('chat-session-list');
  const newBtn = document.getElementById('chat-new-btn');
  const deleteBtn = document.getElementById('chat-delete-btn');
  const kbBtn = document.getElementById('chat-to-kb-btn');

  function renderSessions() {
    const fragment = document.createDocumentFragment();
    for (const [id, s] of sessions) {
      const item = document.createElement('div');
      item.className = 'chat-session-item' + (id === activeSession.id ? ' active' : '');
      const title = document.createElement('div');
      title.className = 'chat-session-title';
      title.textContent = s.title;
      const meta = document.createElement('div');
      meta.className = 'chat-session-meta';
      meta.textContent = `${new Date(s.createdAt).toLocaleDateString()} \u00B7 ${s.messages.filter(m => m.role !== 'system').length} msgs`;
      item.append(title, meta);
      item.addEventListener('click', () => switchSession(id));
      fragment.appendChild(item);
    }
    sessionList.replaceChildren(fragment);
  }

  function switchSession(id) {
    const s = sessions.get(id);
    if (!s) return;
    if (activeSession && activeSession.id !== id) persistSessions();
    activeSession = s;
    renderMessages();
    renderSessions();
  }

  function newSession() {
    const s = {
      id: crypto.randomUUID(),
      title: 'New Chat',
      messages: [{ role: 'system', content: CORE_IDENTITY }],
      createdAt: Date.now(),
      updatedAt: Date.now(),
    };
    sessions.set(s.id, s);
    activeSession = s;
    renderMessages();
    renderSessions();
    persistSessions();
  }

  async function deleteSession() {
    if (sessions.size <= 1) return;
    if (!confirm('Delete "' + activeSession.title + '"?')) return;
    const id = activeSession.id;
    sessions.delete(id);
    activeSession = [...sessions.values()][sessions.size - 1];
    renderMessages();
    renderSessions();
    if (hasDesktopBackend()) {
      invoke('delete_chat_session', { sessionId: id })
        .catch(e => console.warn('delete session:', e));
    }
  }

  async function sendToKb() {
    const msgs = activeSession.messages.filter(m => m.role !== 'system');
    if (msgs.length < 2) {
      events.emit('status-log', { type: 'info', message: 'Not enough messages to extract knowledge.' });
      return;
    }
    if (!confirm(`Extract knowledge from "${activeSession.title}" (${msgs.length} messages)?\n\nThis will analyze the conversation and add entities and relations to the knowledge base.`)) return;

    // Build a combined text from all messages
    const text = msgs.map(m => `[${m.role}]: ${m.content}`).join('\n\n');
    const source = activeSession.title.replace(/[^a-zA-Z0-9_-]/g, '_').slice(0, 40);

    events.emit('status-log', { type: 'info', message: 'Extracting knowledge from chat session...' });
    try {
      const result = await invoke('ingest_text', { text, source, sourceType: 'chat' });
      events.emit('status-log', { type: 'success', message: 'Chat KB: ' + result });
      events.emit('kb-refresh');
    } catch (e) {
      events.emit('status-log', { type: 'error', message: 'Chat KB failed: ' + e });
    }
  }

  function autoTitle() {
    if (activeSession.title !== 'New Chat') return;
    const firstUser = activeSession.messages.find(m => m.role === 'user');
    if (firstUser) {
      activeSession.title = firstUser.content.slice(0, 40) + (firstUser.content.length > 40 ? '...' : '');
      renderSessions();
    }
  }

  function renderMessages() {
    const fragment = document.createDocumentFragment();
    for (const msg of activeSession.messages) {
      if (msg.role === 'system') continue;
      const div = document.createElement('div');
      div.className = 'msg ' + (msg.role === 'assistant' ? 'model' : msg.role);
      // Markdown-render assistant/model/system messages (issue H2: stored role is 'assistant',
      // which previously fell through to the textContent branch and showed raw markdown).
      if (msg.role === 'model' || msg.role === 'assistant' || msg.role === 'system') {
        div.innerHTML = renderMarkdown(msg.content);
      } else {
        div.textContent = msg.content;
      }
      fragment.appendChild(div);
    }
    msgContainer.replaceChildren(fragment);
    msgContainer.scrollTop = msgContainer.scrollHeight;
  }

  function addMessage(role, content) {
    activeSession.messages.push({ role, content, timestamp: Date.now() });
    activeSession.updatedAt = Date.now();
  }

  // Strip expression tag from text, return the display content and expression.
  function stripTag(text) {
    const m = text.match(/^\[(amazed|angry|cry|love|nervous|sleepy|customise|neutral)\]\s*/i);
    if (m) {
      const exprName = EXPR_MAP[m[1].toLowerCase()];
      const stripped = text.slice(m[0].length);
      return { displayText: stripped, exprName };
    }
    return { displayText: text, exprName: null };
  }

  async function sendMessage() {
    const query = inputEl.value.trim();
    if (!query || streaming) return;
    inputEl.value = '';
    inputEl.style.height = 'auto';

    addMessage('user', query);
    const userDiv = document.createElement('div');
    userDiv.className = 'msg user';
    userDiv.textContent = query;
    msgContainer.appendChild(userDiv);
    msgContainer.scrollTop = msgContainer.scrollHeight;
    autoTitle();

    const thinkDiv = document.createElement('div');
    thinkDiv.className = 'msg model streaming';
    thinkDiv.textContent = '\u23F3';
    msgContainer.appendChild(thinkDiv);

    sendBtn.disabled = false;
    sendBtn.textContent = '\u25A0';
    sendBtn.title = 'Stop response';
    streaming = true;
    streamCancelRequested = false;

    // Enrich through the shared Rust backend when a desktop host is present.
    let systemPrompt = CORE_IDENTITY;
    if (hasDesktopBackend()) {
      try {
        const result = await invoke('process_message', { query, basePrompt: systemPrompt });
        if (result.system_prompt) systemPrompt = result.system_prompt;
        if (result.tool_status) {
          addMessage('system', result.tool_status);
        }
      } catch (e) { /* fallback */ }
    }

    const msgs = [
      { role: 'system', content: systemPrompt },
      ...activeSession.messages.filter(m => m.role !== 'system').slice(-20).map(m => ({ role: m.role, content: m.content })),
    ];

    try {
      let raw = '', tagDone = false, displayText = '';
      let exprApplied = false;
      let renderScheduled = false;
      let lastRenderAt = 0;
      const speechStream = new MarkdownSpeechStream();
      thinkDiv.textContent = '';

      const scheduleSpeech = (content) => {
        for (const segment of speechStream.push(content)) {
          events.emit('tts-schedule', segment);
        }
      };

      // Markdown parsing grows with the whole response. Cap it at 20fps so long
      // streams do not monopolize the webview's render thread.
      function scheduleRender() {
        if (renderScheduled) return;
        renderScheduled = true;
        requestAnimationFrame(() => {
          const wait = Math.max(0, 50 - (performance.now() - lastRenderAt));
          setTimeout(() => {
            thinkDiv.innerHTML = renderMarkdown(displayText);
            msgContainer.scrollTop = msgContainer.scrollHeight;
            lastRenderAt = performance.now();
            renderScheduled = false;
          }, wait);
        });
      }

      function consumeDelta(content) {
        if (!content) return;
        raw += content;

        if (!tagDone) {
          const lb = raw.indexOf(']');
          if (raw.startsWith('[') && lb > 0 && lb < 20) {
            const tag = raw.slice(1, lb).toLowerCase();
            if (EXPR_TAGS.includes(tag)) {
              if (!exprApplied && applyExpressionFn) {
                const exprName = EXPR_MAP[tag];
                if (exprName) applyExpressionFn(exprName);
                else if (tag === 'neutral') applyExpressionFn(null);
                exprApplied = true;
              }
              displayText = raw.slice(lb + 1).replace(/^\s+/, '');
              scheduleSpeech(displayText);
              tagDone = true;
              scheduleRender();
              return;
            }
          }
          if (!raw.startsWith('[') && raw.length > 2) {
            if (!exprApplied && applyExpressionFn) { applyExpressionFn(null); exprApplied = true; }
            tagDone = true;
            displayText = raw;
            scheduleSpeech(raw);
            thinkDiv.textContent = raw;
            return;
          }
          if (raw.length > 25) {
            if (!exprApplied && applyExpressionFn) { applyExpressionFn(null); exprApplied = true; }
            tagDone = true;
            displayText = raw;
            scheduleSpeech(raw);
            thinkDiv.textContent = raw;
            return;
          }
        } else {
          displayText += content;
          scheduleSpeech(content);
          scheduleRender();
        }
      }

      streamOperation = invokeStreamCancelable('stream_chat', { messages: msgs }, (event) => {
        if (event?.type === 'delta') consumeDelta(event.content);
      });
      if (streamCancelRequested) await streamOperation.cancel().catch(() => false);
      await streamOperation.promise;

      // Handle any remaining tag (non-streaming or final response)
      if (!tagDone) {
        const m = raw.match(/^\[(amazed|angry|cry|love|nervous|sleepy|customise|neutral)\]\s*/i);
        if (m) {
          if (!exprApplied && applyExpressionFn) {
            const exprName = EXPR_MAP[m[1].toLowerCase()];
            if (exprName) applyExpressionFn(exprName);
            else applyExpressionFn(null);
            exprApplied = true;
          }
          displayText = raw.slice(m[0].length);
          scheduleSpeech(displayText);
        } else {
          displayText = raw;
          scheduleSpeech(displayText);
        }
      }
      thinkDiv.innerHTML = displayText ? renderMarkdown(displayText) : '';

      if (!thinkDiv.textContent.trim() && !thinkDiv.innerHTML.trim()) {
        thinkDiv.textContent = '(no response)';
      }

      // Store the cleaned response (strip any residual expression tag — issue M1).
      const raw0Final = displayText.trim() || thinkDiv.textContent.trim() || raw.trim();
      const finalText = stripTag(raw0Final).displayText.trim();
      addMessage('assistant', finalText);

      // Flush the last prose fragment after the stream completes. Code, math,
      // tables, and incomplete Markdown regions remain suppressed.
      for (const segment of speechStream.finish()) {
        events.emit('tts-schedule', segment);
      }

      // Process turn in background
      if (hasDesktopBackend() && finalText) {
        invoke('process_turn', { userMsg: query, assistantMsg: finalText }).catch(() => {});
      }

    } catch (e) {
      if (e.name === 'AbortError' || e.code === 'request_cancelled') {
        thinkDiv.textContent = '(cancelled)';
      } else {
        thinkDiv.textContent = 'Error: ' + e.message;
        thinkDiv.style.color = '#f87171';
      }
    } finally {
      streaming = false;
      streamOperation = null;
      streamCancelRequested = false;
      sendBtn.disabled = false;
      sendBtn.textContent = '\u2191';
      sendBtn.title = 'Send';
      inputEl.focus();
      persistSessions();
    }
  }

  // Listen for process notifications → inject system messages
  events.on('process-notify', ({ phase, process, data }) => {
    const msg = generateAliceResponse(phase, process, data);
    if (msg) {
      // Strip tag from system messages too
      const { displayText } = stripTag(msg);
      const div = document.createElement('div');
      div.className = 'msg system';
      div.textContent = displayText;
      msgContainer.appendChild(div);
      msgContainer.scrollTop = msgContainer.scrollHeight;
      activeSession.messages.push({ role: 'system', content: displayText, timestamp: Date.now() });
    }
  });

  inputEl.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMessage(); }
  });
  inputEl.addEventListener('input', () => {
    inputEl.style.height = 'auto';
    inputEl.style.height = Math.min(120, inputEl.scrollHeight) + 'px';
  });
  sendBtn.addEventListener('click', () => {
    if (streaming) {
      streamCancelRequested = true;
      sendBtn.disabled = true;
      sendBtn.textContent = '\u23F3';
      void streamOperation?.cancel().catch(() => false);
      return;
    }
    sendMessage();
  });
  newBtn.addEventListener('click', newSession);
  deleteBtn.addEventListener('click', deleteSession);
  kbBtn.addEventListener('click', sendToKb);

  renderSessions();
  renderMessages();

  // Save session periodically and on switch
  async function persistSessions() {
    if (!hasDesktopBackend()) return;
    const all = [];
    for (const [id, s] of sessions) {
      all.push({ id, title: s.title, messages: s.messages.filter(m => m.role !== 'system') });
    }
    try {
      for (const s of all) {
        await invoke('save_chat_session', { sessionId: s.id, title: s.title, messagesJson: JSON.stringify(s.messages) });
      }
    } catch (e) { /* ignore */ }
  }

  // Restore saved sessions on init (retry if DB not ready)
  async function restoreSessions() {
    if (!hasDesktopBackend()) return;
    const tryLoad = async () => {
      try {
        const saved = await invoke('load_chat_sessions');
        if (Array.isArray(saved) && saved.length > 0) {
          sessions.clear();
          for (const s of saved) {
            const session = {
              id: s.id,
              title: s.title,
              messages: [{ role: 'system', content: CORE_IDENTITY }, ...(s.messages || [])],
              createdAt: Date.now(),
              updatedAt: Date.now(),
            };
            sessions.set(s.id, session);
          }
          activeSession = [...sessions.values()][0];
          renderMessages();
          renderSessions();
          return true;
        }
      } catch (e) { /* ignore initial failures */ }
      return false;
    };
    // Try up to 10 times with 500ms delay (DB boots async)
    for (let i = 0; i < 10; i++) {
      try {
        const ready = await invoke('check_db_ready');
        if (ready) { await tryLoad(); return; }
      } catch {}
      await new Promise(r => setTimeout(r, 500));
    }
  }

  restoreSessions();

  // Auto-save every 30 seconds
  setInterval(persistSessions, 30000);
  window.addEventListener('beforeunload', () => {
    persistSessions();
  });

  return panel;
}
