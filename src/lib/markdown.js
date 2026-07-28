// ── Markdown renderer ──────────────────────────────────────

import { marked } from 'marked';

marked.setOptions({ breaks: true, gfm: true });

const renderer = new marked.Renderer();

renderer.code = function (code, language) {
  const lang = language ? `<span class="code-lang">${language}</span>` : '';
  const escaped = code
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
  return `<div class="code-block">
    <div class="code-header">${lang}<button class="copy-btn" data-code="${encodeURIComponent(code)}">Copy</button></div>
    <pre><code class="language-${language || ''}">${escaped}</code></pre>
  </div>`;
};

renderer.link = function (href, title, text) {
  return `<a href="${href}" title="${title || ''}" target="_blank" rel="noopener">${text}</a>`;
};

export function renderMarkdown(text) {
  return marked.parse(text, { renderer });
}

export function renderInline(text) {
  return marked.parseInline(text);
}

// Global copy button handler
document.addEventListener('click', (e) => {
  const btn = e.target.closest('.copy-btn');
  if (!btn) return;
  const code = decodeURIComponent(btn.dataset.code);
  navigator.clipboard.writeText(code).then(() => {
    btn.textContent = 'Copied!';
    setTimeout(() => (btn.textContent = 'Copy'), 1500);
  });
});
