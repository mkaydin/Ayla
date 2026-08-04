// ── Safe DOM helpers ───────────────────────────────────────

/** Escape untrusted text before interpolating it into an HTML template. */
export function escapeHtml(value) {
  return String(value ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/** Restrict a value used as a CSS class to a harmless identifier. */
export function safeCssToken(value, fallback = 'Other') {
  const token = String(value ?? '').replace(/[^a-zA-Z0-9_-]/g, '');
  return token || fallback;
}
