// ── Markdown renderer ──────────────────────────────────────
// marked v18+ token-based API, with DOMPurify sanitization.
//
// SECURITY: All HTML rendered through this module is sanitized with DOMPurify before
// being inserted into the DOM. This is critical because the input is LLM/OCR/search
// output rendered in a Tauri webview that has native IPC access (issues C2/H10/H11).

import { marked } from 'marked';
import DOMPurify from 'dompurify';

marked.setOptions({ breaks: true, gfm: true });

const renderer = new marked.Renderer();

/** Escape a string for use as HTML text content. */
function escapeHtml(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Allow only safe URL schemes (issue H10). Blocks javascript:, data:, vbscript: etc. */
function safeHref(href) {
  const h = String(href || '').trim();
  if (!h) return '';
  // Relative, anchor, and known-safe schemes.
  if (/^(#|\/|mailto:|tel:)/i.test(h)) return h;
  if (/^https?:/i.test(h)) return h;
  // Block everything else (javascript:, data:, vbscript:, file:, …).
  return '';
}

/** Whitelist code-language characters to prevent attribute breakout (issue H11). */
function safeLang(lang) {
  return String(lang || '').replace(/[^a-zA-Z0-9+#.\-_]/g, '').slice(0, 24);
}

renderer.code = function (token) {
  const code = token.text || '';
  const language = safeLang(token.lang);
  const langLabel = language ? `<span class="code-lang">${escapeHtml(language)}</span>` : '';
  const escaped = escapeHtml(code);
  const dataCode = encodeURIComponent(code);
  return `<div class="code-block">
    <div class="code-header">${langLabel}<button class="copy-btn" data-code="${dataCode}">Copy</button></div>
    <pre><code class="language-${escapeHtml(language)}">${escaped}</code></pre>
  </div>`;
};

renderer.link = function (token) {
  const href = safeHref(token.href);
  if (!href) {
    // Disallowed scheme: render the text only, no anchor.
    return escapeHtml(token.text || '');
  }
  const title = token.title ? ` title="${escapeHtml(token.title)}"` : '';
  return `<a href="${escapeHtml(href)}"${title} target="_blank" rel="noopener noreferrer">${escapeHtml(token.text || '')}</a>`;
};

// Allow copy buttons + our code-block markup through sanitization.
const SANITIZE_CONFIG = {
  ALLOWED_TAGS: [
    'a', 'abbr', 'b', 'blockquote', 'br', 'button', 'code', 'div', 'em', 'h1', 'h2', 'h3',
    'h4', 'h5', 'h6', 'hr', 'i', 'img', 'li', 'mark', 'ol', 'p', 'pre', 'span', 'strong',
    'sub', 'sup', 'table', 'tbody', 'td', 'th', 'thead', 'tr', 'u', 'ul', 'del',
  ],
  ALLOWED_ATTR: ['href', 'title', 'target', 'rel', 'class', 'data-code', 'src', 'alt'],
  ALLOW_DATA_ATTR: false,
};

export function renderMarkdown(text) {
  if (typeof text !== 'string') return '';
  const raw = marked.parse(text, { renderer });
  return DOMPurify.sanitize(raw, SANITIZE_CONFIG);
}

export function renderInline(text) {
  if (typeof text !== 'string') return '';
  const raw = marked.parseInline(text);
  return DOMPurify.sanitize(raw, SANITIZE_CONFIG);
}

// Copy-button handler. Attached once as document-level event delegation.
let copyHandlerInstalled = false;
export function installCopyHandler() {
  if (copyHandlerInstalled) return;
  copyHandlerInstalled = true;
  document.addEventListener('click', (e) => {
    const btn = e.target.closest && e.target.closest('.copy-btn');
    if (!btn) return;
    const code = decodeURIComponent(btn.dataset.code || '');
    navigator.clipboard.writeText(code).then(() => {
      btn.textContent = 'Copied!';
      setTimeout(() => (btn.textContent = 'Copy'), 1500);
    });
  });
}

installCopyHandler();
