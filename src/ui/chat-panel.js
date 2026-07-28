// ── Chat Panel ─────────────────────────────────────────────

import { createPanel, getPanel } from './panel-system.js';
import { invoke, getIpc } from '../lib/ipc.js';
import { renderMarkdown } from '../lib/markdown.js';
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

  function renderSessions() {
    sessionList.innerHTML = '';
    for (const [id, s] of sessions) {
      const item = document.createElement('div');
      item.className = 'chat-session-item' + (id === activeSession.id ? ' active' : '');
      item.innerHTML = `
        <div class="chat-session-title">${s.title}</div>
        <div class="chat-session-meta">${new Date(s.createdAt).toLocaleDateString()} \u00B7 ${s.messages.filter(m => m.role !== 'system').length} msgs</div>
      `;
      item.addEventListener('click', () => switchSession(id));
      sessionList.appendChild(item);
    }
  }

  function switchSession(id) {
    const s = sessions.get(id);
    if (!s) return;
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
  }

  function deleteSession() {
    if (sessions.size <= 1) return;
    if (!confirm('Delete "' + activeSession.title + '"?')) return;
    sessions.delete(activeSession.id);
    activeSession = [...sessions.values()][sessions.size - 1];
    renderMessages();
    renderSessions();
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
    msgContainer.innerHTML = '';
    for (const msg of activeSession.messages) {
      if (msg.role === 'system') continue;
      const div = document.createElement('div');
      div.className = 'msg ' + msg.role;
      if (msg.role === 'model' || msg.role === 'system') {
        div.innerHTML = renderMarkdown(msg.content);
      } else {
        div.textContent = msg.content;
      }
      msgContainer.appendChild(div);
    }
    msgContainer.scrollTop = msgContainer.scrollHeight;
  }

  function addMessage(role, content) {
    activeSession.messages.push({ role, content, timestamp: Date.now() });
    activeSession.updatedAt = Date.now();
  }

  const extractSentences = (text, cursor) => {
    const tail = text.slice(cursor);
    const re = /[^.!?\n]+[.!?\n]+/g;
    const sentences = [];
    let m, newCursor = cursor;
    while ((m = re.exec(tail)) !== null) {
      sentences.push(m[0].trim());
      newCursor = cursor + m.index + m[0].length;
    }
    const leftover = text.slice(newCursor);
    if (leftover.length > 90) {
      const lastSpace = leftover.slice(0, 90).lastIndexOf(' ');
      const splitAt = lastSpace > 30 ? lastSpace : 85;
      sentences.push(leftover.slice(0, splitAt).trim());
      newCursor += splitAt;
    }
    return { sentences, cursor: newCursor };
  };

  // Strip expression tag from text, return { displayText, ttsText, exprName }
  function stripTag(text) {
    const m = text.match(/^\[(amazed|angry|cry|love|nervous|sleepy|customise|neutral)\]\s*/i);
    if (m) {
      const exprName = EXPR_MAP[m[1].toLowerCase()];
      const stripped = text.slice(m[0].length);
      return { displayText: stripped, ttsText: stripped, exprName };
    }
    return { displayText: text, ttsText: text, exprName: null };
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

    sendBtn.disabled = true;
    streaming = true;

    // Enrich prompt via Tauri
    let systemPrompt = CORE_IDENTITY;
    const ipcFn = getIpc();
    if (ipcFn) {
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
      const r = await fetch('/api/ollama/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          model: 'gemma4:e4b', messages: msgs, stream: true,
          options: { temperature: 0.7 },
        }),
      });
      if (!r.ok) throw new Error('Ollama: ' + r.status);

      const reader = r.body.getReader();
      const decoder = new TextDecoder();
      let raw = '', buffer = '', tagDone = false, displayText = '', ttsText = '', ttsSent = 0;
      let exprApplied = false;
      let renderScheduled = false;
      thinkDiv.textContent = '';

      // Debounced markdown render — only once per animation frame
      function scheduleRender() {
        if (renderScheduled) return;
        renderScheduled = true;
        requestAnimationFrame(() => {
          thinkDiv.innerHTML = renderMarkdown(displayText);
          msgContainer.scrollTop = msgContainer.scrollHeight;
          renderScheduled = false;
        });
      }

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
                  if (!exprApplied && applyExpressionFn) {
                    const exprName = EXPR_MAP[tag];
                    if (exprName) applyExpressionFn(exprName);
                    else if (tag === 'neutral') applyExpressionFn(null);
                    exprApplied = true;
                  }
                  displayText = raw.slice(lb + 1).replace(/^\s+/, '');
                  ttsText = displayText;
                  tagDone = true;
                  scheduleRender();
                  continue;
                }
              }
              if (!raw.startsWith('[') && raw.length > 2) {
                if (!exprApplied && applyExpressionFn) { applyExpressionFn(null); exprApplied = true; }
                tagDone = true;
                displayText = raw;
                ttsText = raw;
                thinkDiv.textContent = raw;
                continue;
              }
              if (raw.length > 25 && !tagDone) {
                if (!exprApplied && applyExpressionFn) { applyExpressionFn(null); exprApplied = true; }
                tagDone = true;
                displayText = raw;
                ttsText = raw;
                thinkDiv.textContent = raw;
                continue;
              }
            } else {
              displayText += c.message.content;
              ttsText += c.message.content;
              scheduleRender();
            }

            // TTS scheduling — uses displayText (without tag)
            if (tagDone && ttsText.length > ttsSent) {
              const { sentences, cursor } = extractSentences(ttsText, ttsSent);
              for (const s of sentences) {
                events.emit('tts-schedule', s);
              }
              ttsSent = cursor;
            }

          } catch (e) { /* skip malformed lines */ }
        }
      }

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
          ttsText = displayText;
        } else {
          displayText = raw;
          ttsText = raw;
        }
        thinkDiv.innerHTML = displayText ? renderMarkdown(displayText) : '';
      }

      if (!thinkDiv.textContent.trim() && !thinkDiv.innerHTML.trim()) {
        thinkDiv.textContent = '(no response)';
      }

      // Store the cleaned response (without tag)
      const finalText = displayText.trim() || thinkDiv.textContent.trim() || raw.trim();
      addMessage('assistant', finalText);

      // Send remaining TTS
      if (ttsText.length > ttsSent) {
        events.emit('tts-schedule', ttsText.slice(ttsSent).trim());
      }

      // Process turn in background
      if (getIpc() && finalText) {
        invoke('process_turn', { userMsg: query, assistantMsg: finalText }).catch(() => {});
      }

    } catch (e) {
      thinkDiv.textContent = 'Error: ' + e.message;
      thinkDiv.style.color = '#f87171';
    } finally {
      streaming = false;
      sendBtn.disabled = false;
      inputEl.focus();
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
  sendBtn.addEventListener('click', sendMessage);
  newBtn.addEventListener('click', newSession);
  deleteBtn.addEventListener('click', deleteSession);

  renderSessions();
  renderMessages();

  // Save session periodically and on switch
  async function persistSessions() {
    if (!getIpc()) return;
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

  // Restore saved sessions on init
  async function restoreSessions() {
    if (!getIpc()) return;
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
      }
    } catch (e) { /* ignore */ }
  }

  restoreSessions();

  // Persist on session switch
  const origSwitch = switchSession;
  // Auto-save every 30 seconds
  setInterval(persistSessions, 30000);
  window.addEventListener('beforeunload', persistSessions);

  return panel;
}
