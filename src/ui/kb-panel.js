// ── Knowledge Base Panel —— simple Canvas2D graph ───────────

import { createPanel, getPanel } from './panel-system.js';
import { invoke, getIpc } from '../lib/ipc.js';
import { events } from '../lib/events.js';
import { renderMarkdown } from '../lib/markdown.js';

/* ── State ──────────────────────────────────────────────── */
let kbData      = { nodes: [], edges: [], files: [] };
let selectedTab = 'graph';
let selectedNode = null;
let kbPanelRef  = null;

// Graph rendering state
let graphCanvas   = null;
let graphCtx      = null;
let graphNodes    = [];
let graphEdges    = [];
let graphView     = { ox: 0, oy: 0, zoom: 1 };
let panning       = false, panSX = 0, panSY = 0;
let dragging      = false, dragNode = null;
let hoveredNode   = null;
let animId        = null;
let resizeDebounce = null;

// Layout: node positions
const nodePos = {};  // name → { x, y, r, color }

/* ════════════════════════════════════════════════════════════
   Init
   ════════════════════════════════════════════════════════════ */

export function initKbPanel() {
  const panel = createPanel('kb', 'Knowledge Base', {
    x: Math.round(window.innerWidth * 0.15),
    y: Math.round(window.innerHeight * 0.1),
    width: 700, height: 550,
    minWidth: 500, minHeight: 400,
    className: 'kb-panel',
  });
  kbPanelRef = panel;
  loadKbData().then(() => renderKb(panel));

  events.on('kb-refresh', async () => {
    await loadKbData();
    if (kbPanelRef) renderKb(kbPanelRef);
  });

  return panel;
}

async function loadKbData() {
  const ipcFn = getIpc();
  if (ipcFn) {
    try { const data = await invoke('get_kb_data'); if (data) kbData = data; }
    catch (e) { /* offline */ }
  }
  graphNodes = kbData.nodes || [];
  graphEdges = kbData.edges || [];
}

/* ════════════════════════════════════════════════════════════
   Tab shell
   ════════════════════════════════════════════════════════════ */

function renderKb(panel) {
  const body = panel.getContentEl();
  body.innerHTML = `
    <div class="kb-tabs">
      <div class="kb-tab active" data-tab="graph">Graph</div>
      <div class="kb-tab" data-tab="nodes">Nodes</div>
      <div class="kb-tab" data-tab="edges">Edges</div>
      <div class="kb-tab" data-tab="files">Files</div>
    </div>
    <div id="kb-tab-content"></div>
  `;

  body.querySelectorAll('.kb-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      selectedTab = tab.dataset.tab;
      body.querySelectorAll('.kb-tab').forEach(t => t.classList.remove('active'));
      tab.classList.add('active');
      renderTabContent(panel);
    });
  });
  renderTabContent(panel);
}

function renderTabContent(panel) {
  const content = document.getElementById('kb-tab-content');
  if (!content) return;

  // Stop any running animation when leaving graph tab
  if (selectedTab !== 'graph' && animId) {
    cancelAnimationFrame(animId); animId = null;
    graphCanvas = null; graphCtx = null;
  }

  switch (selectedTab) {
    case 'graph': renderGraphTab(content, panel); break;
    case 'nodes': renderNodesTab(content); break;
    case 'edges': renderEdgesTab(content); break;
    case 'files': renderFilesTab(content); break;
  }
}

/* ════════════════════════════════════════════════════════════
   Graph tab — basic Canvas2D, no heavy libraries
   ════════════════════════════════════════════════════════════ */

function renderGraphTab(content, panel) {
  content.innerHTML = `
    <div class="kb-graph-container" id="kb-graph-container">
      <canvas id="kb-graph-canvas"></canvas>
    </div>
    <div class="kb-graph-controls">
      <input class="kb-search-input" id="kb-graph-search" placeholder="Search node..." style="width:140px" />
      <span>Filter:</span>
      <select class="kb-select" id="kb-graph-filter">
        <option value="all">All</option>
        <option value="Person">Person</option>
        <option value="Project">Project</option>
        <option value="Tool">Tool</option>
        <option value="Concept">Concept</option>
        <option value="Document">Document</option>
      </select>
    </div>
    <div class="kb-graph-detail" id="kb-graph-detail">
      <div style="color:var(--text-muted)">Click a node to select.  Scroll to zoom.  Drag to pan.</div>
    </div>
    <div class="kb-graph-actions">
      <button class="ocr-action-btn" id="kb-add-node-btn">+ Add Node</button>
      <button class="ocr-action-btn" id="kb-add-edge-btn">+ Add Edge</button>
      ${selectedNode ? `<button class="ocr-action-btn discard" id="kb-remove-selected-btn">Remove Selected</button>` : ''}
    </div>
  `;

  const canvas = document.getElementById('kb-graph-canvas');
  const container = document.getElementById('kb-graph-container');
  if (!canvas || !container) return;

  graphCanvas = canvas;
  graphCtx = canvas.getContext('2d');

  // Fit canvas to container — debounced to avoid layout-storm on fullscreen/resize
  function fit() {
    const w = container.clientWidth;
    const h = container.clientHeight;
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width  = w;
      canvas.height = h;
    }
  }
  function debouncedFit() {
    fit();
    if (resizeDebounce) clearTimeout(resizeDebounce);
    // Re-layout only after resize settles (300ms), not on every intermediate frame
    resizeDebounce = setTimeout(() => { fit(); runLayout(canvas.width, canvas.height); }, 300);
  }
  new ResizeObserver(debouncedFit).observe(container);
  fit();
  runLayout(canvas.width, canvas.height);

  // ── Interactions ──────────────────────────────────────
  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    const cx = mx / graphView.zoom - graphView.ox;
    const cy = my / graphView.zoom - graphView.oy;
    graphView.zoom *= e.deltaY < 0 ? 1.12 : 1 / 1.12;
    graphView.zoom = Math.max(0.15, Math.min(6, graphView.zoom));
    graphView.ox = mx / graphView.zoom - cx;
    graphView.oy = my / graphView.zoom - cy;
  });

  canvas.addEventListener('pointerdown', (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
    const my = (e.clientY - rect.top)  / graphView.zoom - graphView.oy;
    const hit = hitTest(mx, my);
    if (hit) {
      // Start dragging the node — keep it at its new position when released
      dragging = true;
      dragNode = hit;
      selectNode(hit);
      canvas.setPointerCapture(e.pointerId);
      e.preventDefault();
    } else {
      panning = true;
      panSX = e.clientX; panSY = e.clientY;
    }
  });

  canvas.addEventListener('pointermove', (e) => {
    const rect = canvas.getBoundingClientRect();
    if (dragging && dragNode) {
      const p = nodePos[dragNode.name];
      if (p) {
        p.x = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
        p.y = (e.clientY - rect.top)  / graphView.zoom - graphView.oy;
      }
    } else if (panning) {
      const dx = e.clientX - panSX, dy = e.clientY - panSY;
      graphView.ox += dx / graphView.zoom;
      graphView.oy += dy / graphView.zoom;
      panSX = e.clientX; panSY = e.clientY;
    } else {
      const mx = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
      const my = (e.clientY - rect.top)  / graphView.zoom - graphView.oy;
      hoveredNode = hitTest(mx, my);
      canvas.style.cursor = hoveredNode ? 'pointer' : 'grab';
    }
  });

  canvas.addEventListener('pointerup', () => { dragging = false; dragNode = null; panning = false; });
  canvas.addEventListener('pointerleave', () => { dragging = false; dragNode = null; panning = false; hoveredNode = null; });

  // Controls
  document.getElementById('kb-graph-search')?.addEventListener('input', (e) => {
    const q = e.target.value.toLowerCase();
    const hit = graphNodes.find(n => n.name.toLowerCase().includes(q));
    if (hit && nodePos[hit.name]) {
      const p = nodePos[hit.name];
      graphView.ox = canvas.width / 2 / graphView.zoom - p.x;
      graphView.oy = canvas.height / 2 / graphView.zoom - p.y;
    }
  });
  document.getElementById('kb-graph-filter')?.addEventListener('change', () => {}); // re-render on next frame
  document.getElementById('kb-add-node-btn')?.addEventListener('click', addNodeDialog);
  document.getElementById('kb-add-edge-btn')?.addEventListener('click', addEdgeDialog);
  document.getElementById('kb-remove-selected-btn')?.addEventListener('click', removeSelectedNode);

  runLayout(canvas.width, canvas.height);
  startRenderLoop();
}

/* ── Simple concentric-circle layout ──────────────────────── */
function runLayout(w, h) {
  if (!graphNodes.length) return;

  const cx = w / 2, cy = h / 2;
  const COLORS = ['#e8a87c','#c48cd4','#7ec8a0','#e878a0','#68b8e8','#f0b860','#a0c870','#d4a574','#88c0a0','#c8a0d8'];
  const DEFAULT_COLOR = '#5a5060';

  // ── Colour by source (same document → same colour) ──
  const sourceGroups = {};
  const sourceColor = {};
  let sci = 0;
  for (const n of graphNodes) {
    const src = n.source || '_none';
    if (!sourceGroups[src]) sourceGroups[src] = [];
    sourceGroups[src].push(n);
  }
  for (const src of Object.keys(sourceGroups).sort()) {
    sourceColor[src] = src === '_none' ? DEFAULT_COLOR : COLORS[sci % COLORS.length];
    sci++;
  }

  // ── Compute degree ───────────────────────────────────
  const deg = {};
  for (const n of graphNodes) deg[n.name] = 0;
  for (const e of graphEdges) { deg[e.from] = (deg[e.from] || 0) + 1; deg[e.to] = (deg[e.to] || 0) + 1; }

  // ── Sort by degree descending ────────────────────────
  const sorted = [...graphNodes].sort((a, b) => (deg[b.name] || 0) - (deg[a.name] || 0));

  // ── Place in concentric rings ────────────────────────
  const RING_SIZES = [4, 8, 14, 22, 34];
  let idx = 0;
  for (let ring = 0; ring < RING_SIZES.length && idx < sorted.length; ring++) {
    const count = Math.min(RING_SIZES[ring], sorted.length - idx);
    const radius = 80 + ring * 110;
    for (let i = 0; i < count; i++) {
      const angle = (2 * Math.PI * i) / count - Math.PI / 2;
      const n = sorted[idx];
      // Preserve existing position if node was manually dragged
      if (!nodePos[n.name]) {
        const r = 6 + Math.log2((deg[n.name] || 0) + 1) * 2.5;
        nodePos[n.name] = {
          x: cx + Math.cos(angle) * radius,
          y: cy + Math.sin(angle) * radius,
          r: Math.max(6, Math.min(22, r)),
          color: sourceColor[n.source || '_none'],
        };
      }
      idx++;
    }
  }

  // Update colors on existing nodes (in case source changed)
  for (const n of graphNodes) {
    if (nodePos[n.name]) {
      nodePos[n.name].color = sourceColor[n.source || '_none'];
    }
  }
}

/* ── Hit test ─────────────────────────────────────────────── */
function hitTest(mx, my) {
  for (const n of graphNodes) {
    const p = nodePos[n.name];
    if (!p) continue;
    const dx = mx - p.x, dy = my - p.y;
    if (Math.sqrt(dx * dx + dy * dy) < p.r + 4) return n;
  }
  return null;
}

function selectNode(node) {
  selectedNode = node;
  const detail = document.getElementById('kb-graph-detail');
  if (detail && node) {
    const edges = graphEdges.filter(e => e.from === node.name || e.to === node.name);
    detail.innerHTML = `
      <div class="kb-graph-detail-name">${node.name}</div>
      <div>Type: <span class="type-badge ${node.type || 'Other'}">${node.type || 'Other'}</span></div>
      <div>Degree: ${edges.length}</div>
      <div>Relations: ${edges.map(e => e.relation).join(', ') || 'none'}</div>
    `;
  }
  // Update the remove button
  const actions = document.querySelector('.kb-graph-actions');
  if (actions) {
    const existing = document.getElementById('kb-remove-selected-btn');
    if (node && !existing) {
      const btn = document.createElement('button');
      btn.className = 'ocr-action-btn discard';
      btn.id = 'kb-remove-selected-btn';
      btn.textContent = 'Remove Selected';
      btn.addEventListener('click', () => removeSelectedNode());
      actions.appendChild(btn);
    } else if (!node && existing) {
      existing.remove();
    }
  }
}

/* ── Render loop ──────────────────────────────────────────── */
function startRenderLoop() {
  if (animId) cancelAnimationFrame(animId);
  function frame() {
    if (!graphCanvas || !graphCanvas.isConnected) { animId = null; return; }
    draw();
    animId = requestAnimationFrame(frame);
  }
  animId = requestAnimationFrame(frame);
}

function draw() {
  const ctx = graphCtx;
  const canvas = graphCanvas;
  if (!ctx || !canvas) return;
  const w = canvas.width, h = canvas.height;

  // Background
  ctx.fillStyle = '#0f0d16';
  ctx.fillRect(0, 0, w, h);

  const filter = document.getElementById('kb-graph-filter')?.value || 'all';

  ctx.save();
  ctx.translate(graphView.ox * graphView.zoom, graphView.oy * graphView.zoom);
  ctx.scale(graphView.zoom, graphView.zoom);

  // ── Draw edges ────────────────────────────────────────
  ctx.strokeStyle = 'rgba(150,140,170,0.25)';
  ctx.lineWidth = 1.2;
  for (const e of graphEdges) {
    if (filter !== 'all') {
      const fn = graphNodes.find(n => n.name === e.from);
      const tn = graphNodes.find(n => n.name === e.to);
      if ((!fn || fn.type !== filter) && (!tn || tn.type !== filter)) continue;
    }
    const fp = nodePos[e.from], tp = nodePos[e.to];
    if (!fp || !tp) continue;

    const mx = (fp.x + tp.x) / 2, my = (fp.y + tp.y) / 2;
    const dx = tp.x - fp.x, dy = tp.y - fp.y;
    const len = Math.sqrt(dx * dx + dy * dy) || 1;
    const nx = (-dy / len) * 14, ny = (dx / len) * 14;

    ctx.beginPath();
    ctx.moveTo(fp.x, fp.y);
    ctx.quadraticCurveTo(mx + nx, my + ny, tp.x, tp.y);
    ctx.stroke();
  }

  // ── Draw nodes ────────────────────────────────────────
  const hoveredName = hoveredNode?.name;
  for (const n of graphNodes) {
    const p = nodePos[n.name];
    if (!p) continue;
    if (filter !== 'all' && n.type !== filter) continue;

    const isHovered = n.name === hoveredName;
    const isSelected = selectedNode?.name === n.name;
    const r = p.r;

    // Halo around hovered node
    if (isHovered || isSelected) {
      ctx.beginPath();
      ctx.arc(p.x, p.y, r * 1.7, 0, Math.PI * 2);
      ctx.fillStyle = isSelected ? 'rgba(255,255,255,0.12)' : 'rgba(255,255,255,0.06)';
      ctx.fill();
      ctx.strokeStyle = 'rgba(255,255,255,0.5)';
      ctx.lineWidth = isSelected ? 2 : 1;
      ctx.stroke();
    }

    // Body
    ctx.beginPath();
    ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
    ctx.fillStyle = p.color;
    ctx.fill();

    // Label
    if (graphView.zoom > 0.45) {
      ctx.fillStyle = '#e0dde8';
      ctx.font = `bold ${Math.min(11, Math.max(9, 10 * graphView.zoom))}px sans-serif`;
      ctx.textAlign = 'center';
      ctx.fillText(n.name, p.x, p.y + r + 12);
    }
  }

  ctx.restore();
}

/* ════════════════════════════════════════════════════════════
   Nodes tab
   ════════════════════════════════════════════════════════════ */

function renderNodesTab(content) {
  content.innerHTML = `
    <div class="kb-table-toolbar">
      <input class="kb-search-input" id="kb-nodes-search" placeholder="Search nodes..." />
      <select class="kb-select" id="kb-nodes-filter">
        <option value="all">All types</option>
        <option value="Person">Person</option>
        <option value="Project">Project</option>
        <option value="Tool">Tool</option>
        <option value="Concept">Concept</option>
        <option value="Document">Document</option>
      </select>
    </div>
    <div style="overflow-y:auto;flex:1">
      <table class="kb-table">
        <thead><tr><th>Name</th><th>Type</th><th>Edges</th><th>Actions</th></tr></thead>
        <tbody id="kb-nodes-body"></tbody>
      </table>
    </div>
  `;

  function renderNodeTable() {
    const tbody = document.getElementById('kb-nodes-body');
    if (!tbody) return;
    const search = (document.getElementById('kb-nodes-search')?.value || '').toLowerCase();
    const filter = document.getElementById('kb-nodes-filter')?.value || 'all';
    const filtered = graphNodes.filter(n => {
      if (filter !== 'all' && n.type !== filter) return false;
      if (search && !n.name.toLowerCase().includes(search)) return false;
      return true;
    });
    tbody.innerHTML = filtered.map(n => {
      const ec = graphEdges.filter(e => e.from === n.name || e.to === n.name).length;
      return `<tr><td>${n.name}</td><td><span class="type-badge ${n.type||'Other'}">${n.type||'Other'}</span></td><td>${ec}</td><td><button class="kb-delete-btn" data-delete-node="${n.name}">✗</button></td></tr>`;
    }).join('') || '<tr><td colspan="4" style="text-align:center;color:var(--text-muted)">No nodes found</td></tr>';
    tbody.querySelectorAll('[data-delete-node]').forEach(btn => {
      btn.addEventListener('click', () => deleteNode(btn.dataset.deleteNode));
    });
  }
  document.getElementById('kb-nodes-search')?.addEventListener('input', renderNodeTable);
  document.getElementById('kb-nodes-filter')?.addEventListener('change', renderNodeTable);
  renderNodeTable();
}

/* ════════════════════════════════════════════════════════════
   Edges tab
   ════════════════════════════════════════════════════════════ */

function renderEdgesTab(content) {
  content.innerHTML = `
    <div style="overflow-y:auto;flex:1">
      <table class="kb-table">
        <thead><tr><th>From</th><th>Relation</th><th>To</th><th>Conf</th><th>Actions</th></tr></thead>
        <tbody id="kb-edges-body"></tbody>
      </table>
    </div>
  `;
  const tbody = document.getElementById('kb-edges-body');
  if (!tbody) return;
  tbody.innerHTML = graphEdges.map((e, i) => `
    <tr><td>${e.from}</td><td>${e.relation}</td><td>${e.to}</td><td>${(e.confidence||0).toFixed(2)}</td>
    <td><button class="kb-delete-btn" data-delete-edge="${i}">✗</button></td></tr>
  `).join('') || '<tr><td colspan="5" style="text-align:center;color:var(--text-muted)">No edges found</td></tr>';
  tbody.querySelectorAll('[data-delete-edge]').forEach(btn => {
    btn.addEventListener('click', () => deleteEdge(parseInt(btn.dataset.deleteEdge)));
  });
}

/* ════════════════════════════════════════════════════════════
   Files tab
   ════════════════════════════════════════════════════════════ */

function renderFilesTab(content) {
  content.innerHTML = '<div style="text-align:center;padding:24px;color:var(--text-muted)">Loading files...</div>';
  if (getIpc()) {
    invoke('get_files').then(result => {
      kbData.files = Array.isArray(result) ? result : [];
      renderFilesTable(content);
    }).catch(e => {
      kbData.files = [];
      renderFilesTable(content);
    });
  } else { kbData.files = []; renderFilesTable(content); }
}

function renderFilesTable(content) {
  content.innerHTML = `
    <div style="overflow-y:auto;flex:1"><table class="kb-table">
      <thead><tr><th>File</th><th>Chars</th><th>Source</th><th>Date</th><th>Actions</th></tr></thead>
      <tbody id="kb-files-body"></tbody>
    </table></div>`;
  const tbody = document.getElementById('kb-files-body');
  if (!tbody) return;
  const files = kbData.files || [];
  tbody.innerHTML = files.length === 0
    ? '<tr><td colspan="5" style="text-align:center;color:var(--text-muted);padding:32px">No files yet.</td></tr>'
    : files.map(f => `<tr>
      <td>${f.name||'?'}</td><td>${(f.size||0).toLocaleString()}</td><td>${f.source||'-'}</td><td>${(f.date||'').slice(0,10)}</td>
      <td style="white-space:nowrap">
        ${f.path ? `<button class="kb-open-btn" data-open-file="${f.name}">Open</button>` : ''}
        <button class="kb-open-btn" data-view-ocr="${f.name}">View</button>
        <button class="kb-delete-btn" data-delete-file="${f.name}">Delete</button>
      </td></tr>`).join('');
  tbody.querySelectorAll('[data-open-file]').forEach(btn => {
    btn.addEventListener('click', () => {
      const file = files.find(f => f.name === btn.dataset.openFile);
      if (file?.path) import('../lib/ipc.js').then(m => m.openPdf(file.path));
    });
  });
  tbody.querySelectorAll('[data-view-ocr]').forEach(btn => {
    btn.addEventListener('click', async () => {
      const fileName = btn.dataset.viewOcr;
      const file = files.find(f => f.name === fileName);
      const ocrPanel = getPanel('ocr');
      if (!ocrPanel) return;
      ocrPanel.show();
      const body = ocrPanel.getContentEl();
      body.innerHTML = '<div style="padding:32px;color:var(--text-muted);text-align:center">Loading…</div>';
      try {
        const text = file?.text || await invoke('get_document_text', { name: fileName });
        body.innerHTML = `<button class="ocr-back-btn" id="ocr-kb-back-btn">← Back</button><div class="ocr-preview" style="flex:1;overflow-y:auto;padding:12px">${text ? renderMarkdown(text) : '<em>No text found.</em>'}</div>`;
        document.getElementById('ocr-kb-back-btn').addEventListener('click', () => import('./ocr-panel.js').then(m => m.resetOcrView()));
      } catch (e) {
        body.innerHTML = `<button class="ocr-back-btn" id="ocr-kb-back-btn">← Back</button><div style="padding:12px;color:var(--text-secondary)">Failed: ${e}</div>`;
        document.getElementById('ocr-kb-back-btn').addEventListener('click', () => import('./ocr-panel.js').then(m => m.resetOcrView()));
      }
    });
  });
  tbody.querySelectorAll('[data-delete-file]').forEach(btn => {
    btn.addEventListener('click', () => {
      if (confirm(`Delete "${btn.dataset.deleteFile}" and its graph nodes?`)) {
        invoke('delete_file_record', { name: btn.dataset.deleteFile, deleteNodes: true }).then(msg => {
          events.emit('status-log', { type: 'info', message: msg });
          events.emit('kb-refresh');
        }).catch(e => alert('Failed: ' + e));
      }
    });
  });
}

/* ════════════════════════════════════════════════════════════
   CRUD
   ════════════════════════════════════════════════════════════ */

function addNodeDialog() {
  const name = prompt('Node name:'); if (!name) return;
  const type = prompt('Type (Person/Project/Tool/Concept/Document/Other):', 'Concept'); if (!type) return;
  graphNodes.push({ name, type });
  kbData.nodes = graphNodes;
  renderGraphTab(document.getElementById('kb-tab-content'), getPanel('kb'));
  if (getIpc()) invoke('kb_add_node', { name, nodeType: type }).catch(() => {});
}

function addEdgeDialog() {
  const from = prompt('From node:'); if (!from) return;
  const to = prompt('To node:'); if (!to) return;
  const rel = prompt('Relation (USES/PREFERS/...):', 'RELATED_TO'); if (!rel) return;
  graphEdges.push({ from, to, relation: rel, confidence: 0.8 });
  kbData.edges = graphEdges;
  renderGraphTab(document.getElementById('kb-tab-content'), getPanel('kb'));
  if (getIpc()) invoke('kb_add_edge', { from, to, relation: rel, confidence: 0.8 }).catch(() => {});
}

function removeSelectedNode() {
  if (!selectedNode) return;
  if (!confirm(`Remove "${selectedNode.name}" and its edges?`)) return;
  const name = selectedNode.name;
  graphNodes = graphNodes.filter(n => n !== selectedNode);
  graphEdges = graphEdges.filter(e => e.from !== name && e.to !== name);
  kbData.nodes = graphNodes; kbData.edges = graphEdges;
  selectedNode = null;
  if (getIpc()) invoke('kb_delete_node', { name }).catch(() => {});
  renderGraphTab(document.getElementById('kb-tab-content'), getPanel('kb'));
}

function deleteNode(name) {
  if (!confirm(`Delete node "${name}"?`)) return;
  graphNodes = graphNodes.filter(n => n.name !== name);
  graphEdges = graphEdges.filter(e => e.from !== name && e.to !== name);
  kbData.nodes = graphNodes; kbData.edges = graphEdges;
  if (selectedNode?.name === name) selectedNode = null;
  if (getIpc()) invoke('kb_delete_node', { name }).catch(() => {});
  renderGraphTab(document.getElementById('kb-tab-content'), getPanel('kb'));
}

function deleteEdge(index) {
  if (!confirm('Delete this edge?')) return;
  const edge = graphEdges[index];
  graphEdges.splice(index, 1);
  kbData.edges = graphEdges;
  if (getIpc() && edge) invoke('kb_delete_edge', { from: edge.from, to: edge.to, relation: edge.relation||'' }).catch(() => {});
  renderGraphTab(document.getElementById('kb-tab-content'), getPanel('kb'));
}
