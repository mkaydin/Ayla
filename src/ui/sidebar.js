// ── Sidebar ────────────────────────────────────────────────

import { createPanel, getPanel } from './panel-system.js';

const TOGGLE_BUTTONS = [
  { id: 'chat',   icon: '\uD83D\uDCAC', label: 'Chat',   shortcut: 'Ctrl+C' },
  { id: 'ocr',    icon: '\uD83D\uDCC4', label: 'OCR',    shortcut: 'Ctrl+O' },
  { id: 'search', icon: '\uD83D\uDD0D', label: 'Search', shortcut: 'Ctrl+S' },
  { id: 'kb',     icon: '\uD83E\uDDE0', label: 'KB',     shortcut: 'Ctrl+K' },
];

export function initSidebar() {
  const sidebar = document.createElement('div');
  sidebar.className = 'panel-sidebar';

  for (const btn of TOGGLE_BUTTONS) {
    const el = document.createElement('button');
    el.className = 'panel-toggle-btn';
    el.innerHTML = `${btn.icon}<span class="tooltip">${btn.label} (${btn.shortcut})</span>`;
    el.title = `${btn.label} (${btn.shortcut})`;

    el.addEventListener('click', () => {
      const panel = getPanel(btn.id);
      if (panel) {
        if (panel.isVisible) {
          panel.hide();
          el.classList.remove('active');
        } else {
          panel.show();
          el.classList.add('active');
        }
      }
    });

    sidebar.appendChild(el);
  }

  document.body.appendChild(sidebar);
  window.__sidebarEl = sidebar;
  window.__sidebarBtns = TOGGLE_BUTTONS;

  // Keyboard shortcuts
  document.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.key === 'c') { e.preventDefault(); toggleSidebarPanel('chat'); }
    if (e.ctrlKey && e.key === 'o') { e.preventDefault(); toggleSidebarPanel('ocr'); }
    if (e.ctrlKey && e.key === 's') { e.preventDefault(); toggleSidebarPanel('search'); }
    if (e.ctrlKey && e.key === 'k') { e.preventDefault(); toggleSidebarPanel('kb'); }
  });

  return sidebar;
}

function toggleSidebarPanel(id) {
  const panel = getPanel(id);
  if (!panel) return;
  if (panel.isVisible) panel.hide(); else panel.show();
  updateSidebarState();
}

export function setBadge(panelId, show) {
  const sidebar = window.__sidebarEl;
  if (!sidebar || !window.__sidebarBtns) return;
  const idx = window.__sidebarBtns.findIndex(b => b.id === panelId);
  if (idx < 0) return;
  const btn = sidebar.children[idx];
  if (!btn) return;
  if (show) {
    if (!btn.querySelector('.badge')) {
      const badge = document.createElement('span');
      badge.className = 'badge';
      btn.appendChild(badge);
    }
  } else {
    const badge = btn.querySelector('.badge');
    if (badge) badge.remove();
  }
}

export function updateSidebarState() {
  const sidebar = window.__sidebarEl;
  if (!sidebar || !window.__sidebarBtns) return;
  for (let i = 0; i < window.__sidebarBtns.length; i++) {
    const btn = sidebar.children[i];
    const id = window.__sidebarBtns[i].id;
    const panel = getPanel(id);
    if (btn && panel) {
      btn.classList.toggle('active', panel.isVisible);
    }
  }
}
