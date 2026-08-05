// ── Markdown renderer ──────────────────────────────────────
// marked v18+ token-based API, with DOMPurify sanitization.
//
// SECURITY: All HTML rendered through this module is sanitized with DOMPurify before
// being inserted into the DOM. This is critical because the input is LLM/OCR/search
// output rendered in a desktop webview that has native IPC access (issues C2/H10/H11).

import { marked } from 'marked';
import DOMPurify from 'dompurify';
import katex from 'katex';
import hljs from 'highlight.js';

marked.setOptions({ breaks: true, gfm: true });

const renderer = new marked.Renderer();

const MATH_TOKEN_PREFIX = 'AYLA_MATH_TOKEN_';
const CODE_HIGHLIGHT_CACHE_LIMIT = 128;
const codeHighlightCache = new Map();
const AUTO_DETECT_LANGUAGES = [
  'bash', 'c', 'cpp', 'csharp', 'css', 'go', 'html', 'java', 'javascript',
  'json', 'kotlin', 'lua', 'markdown', 'php', 'python', 'ruby', 'rust', 'sql',
  'swift', 'typescript', 'xml', 'yaml',
];

function renderMath(source, displayMode) {
  try {
    return katex.renderToString(source, {
      displayMode,
      throwOnError: false,
      strict: 'ignore',
      trust: false,
      output: 'html',
    });
  } catch {
    return `<code class="math-error">${escapeHtml(source)}</code>`;
  }
}

/**
 * Replaces LaTeX delimiters with opaque text tokens before Marked parses the
 * document. Fenced and inline code are preserved verbatim, so `$value` in a
 * code sample never becomes an equation. KaTeX HTML is injected only after
 * DOMPurify has sanitized the normal Markdown output; KaTeX itself runs with
 * `trust: false` and cannot render user-provided HTML or URLs.
 */
function protectMath(markdown) {
  const rendered = [];
  const tokenFor = (source, displayMode) => {
    const token = `${MATH_TOKEN_PREFIX}${rendered.length}_END`;
    rendered.push({ token, html: renderMath(source.trim(), displayMode) });
    return token;
  };

  const replaceMath = segment => segment
    // $$...$$ and \[...\] are display equations and may span multiple lines.
    .replace(/\$\$([\s\S]*?)\$\$/g, (_all, source) => tokenFor(source, true))
    .replace(/\\\[([\s\S]*?)\\\]/g, (_all, source) => tokenFor(source, true))
    // \(...\) and $...$ are inline equations. A whitespace boundary prevents
    // treating currency such as "$ 25" as mathematics.
    .replace(/\\\(([^\n]*?)\\\)/g, (_all, source) => tokenFor(source, false))
    .replace(/(?<!\\)\$([^$\n]*?\S[^$\n]*?)\$(?!\$)/g, (all, source) => {
      if (/^\s|\s$/.test(source)) return all;
      return tokenFor(source, false);
    });

  // Split fenced code first, then protect inline code while processing prose.
  const fences = markdown.split(/(```[\s\S]*?```|~~~[\s\S]*?~~~)/g);
  const protectedMarkdown = fences.map((part, index) => {
    if (index % 2 === 1) return part;
    return part.split(/(`[^`\n]*`)/g).map((piece, pieceIndex) => (
      pieceIndex % 2 === 1 ? piece : replaceMath(piece)
    )).join('');
  }).join('');
  return { markdown: protectedMarkdown, rendered };
}

function restoreMath(html, rendered) {
  return rendered.reduce(
    (output, { token, html: mathHtml }) => output.replaceAll(token, mathHtml),
    html,
  );
}

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

/** Highlight fenced code safely. Highlight.js escapes all source code itself. */
function highlightCode(code, language) {
  const cacheKey = `${language}\u0000${code}`;
  const cached = codeHighlightCache.get(cacheKey);
  if (cached) return cached;

  // Syntax highlighting an enormous generated file while an answer streams can
  // hurt UI responsiveness. It remains readable and copyable without color.
  if (code.length > 200_000) return escapeHtml(code);

  let highlighted;
  try {
    highlighted = language && hljs.getLanguage(language)
      ? hljs.highlight(code, { language, ignoreIllegals: true }).value
      : hljs.highlightAuto(code, AUTO_DETECT_LANGUAGES).value;
  } catch {
    highlighted = escapeHtml(code);
  }

  if (codeHighlightCache.size >= CODE_HIGHLIGHT_CACHE_LIMIT) {
    codeHighlightCache.delete(codeHighlightCache.keys().next().value);
  }
  codeHighlightCache.set(cacheKey, highlighted);
  return highlighted;
}

renderer.code = function (token) {
  const code = token.text || '';
  const language = safeLang(token.lang);
  const langLabel = language ? `<span class="code-lang">${escapeHtml(language)}</span>` : '';
  const highlighted = highlightCode(code, language);
  const dataCode = encodeURIComponent(code);
  return `<div class="code-block">
    <div class="code-header">${langLabel}<button class="copy-btn" data-code="${dataCode}">Copy</button></div>
    <pre><code class="hljs language-${escapeHtml(language)}">${highlighted}</code></pre>
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
  const protectedMath = protectMath(text);
  const raw = marked.parse(protectedMath.markdown, { renderer });
  return restoreMath(DOMPurify.sanitize(raw, SANITIZE_CONFIG), protectedMath.rendered);
}

export function renderInline(text) {
  if (typeof text !== 'string') return '';
  const protectedMath = protectMath(text);
  const raw = marked.parseInline(protectedMath.markdown);
  return restoreMath(DOMPurify.sanitize(raw, SANITIZE_CONFIG), protectedMath.rendered);
}

/**
 * Convert a Markdown answer into prose suitable for TTS. Code, equations,
 * tables, URLs, and other presentation-only syntax are deliberately omitted.
 * Keeping this separate from renderMarkdown means visual output stays complete
 * while speech remains natural and does not leak Markdown punctuation.
 */
export function markdownToSpeechText(text) {
  if (typeof text !== 'string') return '';
  let prose = text;

  // Remove fenced code, raw code/pre blocks, LaTex environments, and both
  // complete and unfinished display-equation forms before touching formatting.
  prose = prose
    .replace(/```[\s\S]*?(?:```|$)/g, '')
    .replace(/~~~[\s\S]*?(?:~~~|$)/g, '')
    .replace(/<(?:pre|code|table|script|style|svg)\b[^>]*>[\s\S]*?<\/(?:pre|code|table|script|style|svg)>/gi, '')
    .replace(/\$\$[\s\S]*?\$\$/g, '')
    .replace(/\$\$[\s\S]*$/g, '')
    .replace(/\\\[[\s\S]*?\\\]/g, '')
    .replace(/\\\[[\s\S]*$/g, '')
    .replace(/\\begin\{(?:equation\*?|align\*?|gather\*?|multline\*?|matrix|pmatrix|bmatrix|vmatrix|cases)\}[\s\S]*?\\end\{[^}]+\}/g, '')
    .replace(/\\\([^\n]*?\\\)/g, '')
    .replace(/\\\([^\n]*$/g, '')
    .replace(/(?<!\\)\$([^$\n]*?\S[^$\n]*?)\$(?!\$)/g, (all, source) => (
      /^\s|\s$/.test(source) ? all : ''
    ))
    .replace(/(?<!\\)\$(?=\S)[^$\n]*$/g, '');

  // A Markdown table is presentation data, not natural speech. Removing every
  // pipe-containing row also covers separator rows and partially streamed tables.
  prose = prose.split('\n').filter(line => !line.includes('|')).join('\n');

  return prose
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '')
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]+)\]\[[^\]]*\]/g, '$1')
    .replace(/<https?:\/\/[^>]+>/gi, '')
    .replace(/https?:\/\/\S+/gi, '')
    .replace(/`[^`\n]*`/g, '')
    .replace(/`[^`\n]*$/gm, '')
    .replace(/^\s{0,3}#{1,6}\s+/gm, '')
    .replace(/^\s*>\s?/gm, '')
    .replace(/^\s*(?:[-+*]|\d+[.)])\s+(?:\[[ xX]\]\s+)?/gm, '')
    .replace(/^\s*([-*_])(?:\s*\1){2,}\s*$/gm, '')
    .replace(/^\s*\[[^\]]+\]:\s*\S+.*$/gm, '')
    .replace(/<(?:[^>]+)>/g, '')
    .replace(/(\*\*|__|~~)(.*?)\1/g, '$2')
    .replace(/\*([^*\n]+)\*/g, '$1')
    .replace(/_([^_\n]+)_/g, '$1')
    .replace(/\\([\\`*_{}\[\]()#+\-.!>])/g, '$1')
    .replace(/[*_~#]+/g, '')
    .replace(/[\t ]*\n[\t ]*/g, ' ')
    .replace(/\s{2,}/g, ' ')
    .trim();
}

function findUnclosedBlockMath(text) {
  const delimiters = [
    { open: '$$', close: '$$' },
    { open: '\\[', close: '\\]' },
  ];
  let earliest = null;

  for (const delimiter of delimiters) {
    let offset = 0;
    while (offset < text.length) {
      const openAt = text.indexOf(delimiter.open, offset);
      if (openAt < 0) break;
      const closeAt = text.indexOf(delimiter.close, openAt + delimiter.open.length);
      if (closeAt < 0) {
        if (!earliest || openAt < earliest.index) {
          earliest = { index: openAt, close: delimiter.close };
        }
        break;
      }
      offset = closeAt + delimiter.close.length;
    }
  }
  return earliest;
}

/** Find the last completed sentence that is outside Markdown special regions. */
function safeSentenceBoundary(text) {
  if (!text || text.includes('|') || text.includes('```') || text.includes('~~~')) return 0;

  let inlineCode = false;
  let mathEnd = null;
  let linkDepth = 0;
  let boundary = 0;

  for (let i = 0; i < text.length; i += 1) {
    const char = text[i];
    const next = text[i + 1] || '';
    if (char === '\\' && !mathEnd) {
      if (next === '(' || next === '[') {
        mathEnd = next === '(' ? '\\)' : '\\]';
        i += 1;
      } else {
        i += 1;
      }
      continue;
    }
    if (mathEnd) {
      if (text.startsWith(mathEnd, i)) {
        i += mathEnd.length - 1;
        mathEnd = null;
      }
      continue;
    }
    if (char === '`') {
      inlineCode = !inlineCode;
      continue;
    }
    if (inlineCode) continue;
    if (char === '$') {
      mathEnd = next === '$' ? '$$' : '$';
      if (next === '$') i += 1;
      continue;
    }
    if (linkDepth > 0) {
      if (char === '(') linkDepth += 1;
      if (char === ')') linkDepth -= 1;
      continue;
    }
    if (char === ']' && next === '(') {
      linkDepth = 1;
      i += 1;
      continue;
    }
    if (char === '.' || char === '!' || char === '?') {
      let end = i + 1;
      while (/["')\]]/.test(text[end] || '')) end += 1;
      if (end === text.length || /\s/.test(text[end])) boundary = end;
    }
  }
  return boundary;
}

/**
 * Stateful Markdown-to-speech filter for streamed LLM output. It emits complete
 * prose sentences as soon as they are safe, while retaining partial delimiters
 * until the stream proves they are prose rather than code, math, or a table.
 */
export class MarkdownSpeechStream {
  constructor() {
    this.pending = '';
    this.fence = null;
    this.blockMathEnd = null;
  }

  _processLine(rawLine) {
    let line = rawLine;
    const trimmed = line.trimStart();

    if (this.fence) {
      if (trimmed.startsWith(this.fence)) this.fence = null;
      return '';
    }
    if (trimmed.startsWith('```') || trimmed.startsWith('~~~')) {
      this.fence = trimmed.slice(0, 3);
      return '';
    }

    if (this.blockMathEnd) {
      const closeAt = line.indexOf(this.blockMathEnd);
      if (closeAt < 0) return '';
      line = line.slice(closeAt + this.blockMathEnd.length);
      this.blockMathEnd = null;
    }

    const unclosedMath = findUnclosedBlockMath(line);
    if (unclosedMath) {
      const prosePrefix = line.slice(0, unclosedMath.index);
      this.blockMathEnd = unclosedMath.close;
      return markdownToSpeechText(prosePrefix);
    }

    if (line.includes('|')) return '';
    return markdownToSpeechText(line);
  }

  push(chunk) {
    if (typeof chunk !== 'string' || !chunk) return [];
    this.pending += chunk;
    const segments = [];

    let newlineAt = this.pending.indexOf('\n');
    while (newlineAt >= 0) {
      const line = this.pending.slice(0, newlineAt + 1);
      this.pending = this.pending.slice(newlineAt + 1);
      const speech = this._processLine(line);
      if (speech) segments.push(speech);
      newlineAt = this.pending.indexOf('\n');
    }

    if (!this.fence && !this.blockMathEnd) {
      const boundary = safeSentenceBoundary(this.pending);
      if (boundary > 0) {
        const complete = this.pending.slice(0, boundary);
        this.pending = this.pending.slice(boundary);
        const speech = this._processLine(complete);
        if (speech) segments.push(speech);
      }
    }
    return segments;
  }

  finish() {
    const segments = [];
    if (!this.fence && !this.blockMathEnd && this.pending) {
      const speech = this._processLine(this.pending);
      if (speech) segments.push(speech);
    }
    this.pending = '';
    this.fence = null;
    this.blockMathEnd = null;
    return segments;
  }
}

export { protectMath };

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

if (typeof document !== 'undefined') installCopyHandler();
