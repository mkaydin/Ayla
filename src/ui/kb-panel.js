// ── Knowledge Base Panel ───────────────────────────────────

import { createPanel, getPanel } from './panel-system.js';
import { invoke, getIpc } from '../lib/ipc.js';
import { events } from '../lib/events.js';

let kbData = { nodes: [], edges: [], files: [] };
let selectedTab = 'graph';
let graphNodes = [];
let graphEdges = [];
let selectedNode = null;
let canvasCtx = null;
let graphView = { ox: 0, oy: 0, zoom: 1 };
let panning = false, panStartX = 0, panStartY = 0;
let kbPanelRef = null;

export function initKbPanel() {
  const panel = createPanel('kb', 'Knowledge Base', {
    x: Math.round(window.innerWidth * 0.15),
    y: Math.round(window.innerHeight * 0.1),
    width: 700, height: 550,
    minWidth: 500, minHeight: 400,
    className: 'kb-panel',
    onClose: () => {},
  });

  kbPanelRef = panel;
  loadKbData().then(() => renderKb(panel));

  // Listen for refresh events
  events.on('kb-refresh', async () => {
    await loadKbData();
    // Always update graph data, re-render tab content
    if (kbPanelRef) {
      renderKb(kbPanelRef);
      // Force graph redraw after re-render
      setTimeout(() => {
        if (canvasCtx) drawGraph();
      }, 100);
    }
  });

  return panel;
}

async function loadKbData() {
  const ipcFn = getIpc();
  if (ipcFn) {
    try {
      const data = await invoke('get_kb_data');
      if (data) kbData = data;
    } catch (e) { /* offline */ }
  }
  graphNodes = kbData.nodes || [];
  graphEdges = kbData.edges || [];
}

function renderKb(panel) {
  const body = panel.getContentEl();
  body.innerHTML = `
    <div class="kb-tabs">
      <div class="kb-tab active" data-tab="graph">\uD83D\uDCCA Graph</div>
      <div class="kb-tab" data-tab="nodes">\uD83D\uDCCB Nodes</div>
      <div class="kb-tab" data-tab="edges">\uD83D\uDD17 Edges</div>
      <div class="kb-tab" data-tab="files">\uD83D\uDCC1 Files</div>
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

  switch (selectedTab) {
    case 'graph': renderGraphTab(content, panel); break;
    case 'nodes': renderNodesTab(content); break;
    case 'edges': renderEdgesTab(content); break;
    case 'files': renderFilesTab(content); break;
  }
}

/* ── Graph Tab ────────────────────────────────────────────── */

function renderGraphTab(content, panel) {
  content.innerHTML = `
    <div class="kb-graph-container" id="kb-graph-container">
      <canvas class="kb-graph-canvas" id="kb-graph-canvas"></canvas>
    </div>
    <div class="kb-graph-controls">
      <span>Filter:</span>
      <select class="kb-select" id="kb-graph-filter">
        <option value="all">All</option>
        <option value="Person">\uD83D\uDC64 Person</option>
        <option value="Project">\uD83D\uDE80 Project</option>
        <option value="Tool">\uD83D\uDD27 Tool</option>
        <option value="Concept">\uD83D\uDCA1 Concept</option>
        <option value="Document">\uD83D\uDCC4 Document</option>
      </select>
    </div>
    <div class="kb-graph-detail" id="kb-graph-detail">
      ${selectedNode ? `
        <div class="kb-graph-detail-name">${selectedNode.name}</div>
        <div>Type: <span class="type-badge ${selectedNode.type}">${selectedNode.type}</span></div>
        <div>Relations: ${(graphEdges || []).filter(e => e.from === selectedNode.name || e.to === selectedNode.name).map(e => e.relation).join(', ') || 'none'}</div>
      ` : '<div style="color:var(--text-muted)">Click a node to select it. Drag to rearrange.</div>'}
    </div>
    <div class="kb-graph-actions">
      <button class="ocr-action-btn" id="kb-add-node-btn">+ Add Node</button>
      <button class="ocr-action-btn" id="kb-add-edge-btn">+ Add Edge</button>
      ${selectedNode ? `<button class="ocr-action-btn discard" id="kb-remove-selected-btn">Remove Selected</button>` : ''}
    </div>
  `;

  document.getElementById('kb-graph-filter')?.addEventListener('change', () => drawGraph());
  document.getElementById('kb-add-node-btn')?.addEventListener('click', () => addNodeDialog());
  document.getElementById('kb-add-edge-btn')?.addEventListener('click', () => addEdgeDialog());
  document.getElementById('kb-remove-selected-btn')?.addEventListener('click', () => removeSelectedNode());

  // Init canvas with dynamic sizing
  const canvas = document.getElementById('kb-graph-canvas');
  if (!canvas) return;

  const container = document.getElementById('kb-graph-container');
  canvasCtx = canvas.getContext('2d');

  function fitCanvas() {
    const w = container.clientWidth;
    const h = container.clientHeight;
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
      layoutGraph(w, h);
      drawGraph();
    }
  }

  // Initial fit
  fitCanvas();

  // ResizeObserver for dynamic panel resize
  if (window.ResizeObserver) {
    const ro = new ResizeObserver(() => fitCanvas());
    ro.observe(container);
  }

  // Layout nodes
  layoutGraph(canvas.width, canvas.height);

  // Canvas interaction: node drag, view pan, zoom-to-cursor
  let dragging = null, dragNode = null;

  canvas.addEventListener('pointerdown', (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
    const my = (e.clientY - rect.top) / graphView.zoom - graphView.oy;

    const hit = graphNodes.find(n => {
      const dx = mx - n.x, dy = my - n.y;
      return Math.sqrt(dx * dx + dy * dy) < (n.radius || 20);
    });

    if (hit) {
      dragging = true;
      dragNode = hit;
      selectNode(hit);
      drawGraph();
      // Update detail panel without full re-render
      const detail = document.getElementById('kb-graph-detail');
      if (detail) {
        detail.innerHTML = `
          <div class="kb-graph-detail-name">${hit.name}</div>
          <div>Type: <span class="type-badge ${hit.type || 'Other'}">${hit.type || 'Other'}</span></div>
        `;
      }
    } else {
      // Start view pan
      panning = true;
      panStartX = e.clientX;
      panStartY = e.clientY;
    }
  });

  canvas.addEventListener('pointermove', (e) => {
    if (dragging && dragNode) {
      const rect = canvas.getBoundingClientRect();
      dragNode.x = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
      dragNode.y = (e.clientY - rect.top) / graphView.zoom - graphView.oy;
      drawGraph();
    } else if (panning) {
      const dx = e.clientX - panStartX;
      const dy = e.clientY - panStartY;
      graphView.ox += dx / graphView.zoom;
      graphView.oy += dy / graphView.zoom;
      panStartX = e.clientX;
      panStartY = e.clientY;
      drawGraph();
    }
  });

  canvas.addEventListener('pointerup', () => {
    dragging = false; dragNode = null;
    panning = false;
    canvas.style.cursor = '';
  });

  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const worldX = mx / graphView.zoom - graphView.ox;
    const worldY = my / graphView.zoom - graphView.oy;

    graphView.zoom *= e.deltaY < 0 ? 1.15 : 1 / 1.15;
    graphView.zoom = Math.max(0.1, Math.min(5, graphView.zoom));

    // Adjust offset to keep cursor point stationary
    graphView.ox = mx / graphView.zoom - worldX;
    graphView.oy = my / graphView.zoom - worldY;
    drawGraph();
  });

  drawGraph();
}

function layoutGraph(w, h) {
  if (!graphNodes.length) return;

  // Simple circular layout
  const cx = w / 2, cy = h / 2;
  const radius = Math.min(w, h) * 0.35;

  graphNodes.forEach((n, i) => {
    const angle = (2 * Math.PI * i) / graphNodes.length;
    n.x = cx + Math.cos(angle) * radius + (Math.random() - 0.5) * 40;
    n.y = cy + Math.sin(angle) * radius + (Math.random() - 0.5) * 40;
    n.radius = 20;
  });
}

function drawGraph() {
  if (!canvasCtx) return;
  const ctx = canvasCtx;
  const canvas = ctx.canvas;
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  ctx.save();
  ctx.translate(graphView.ox * graphView.zoom, graphView.oy * graphView.zoom);
  ctx.scale(graphView.zoom, graphView.zoom);

  const filter = document.getElementById('kb-graph-filter')?.value || 'all';
  const visibleNodes = filter === 'all' ? graphNodes : graphNodes.filter(n => n.type === filter);
  const visibleNames = new Set(visibleNodes.map(n => n.name));

  // Compute connected components for coloring
  const componentColors = [
    { fill: '#3b82f6', glow: 'rgba(59,130,246,0.3)' },   // blue
    { fill: '#22c55e', glow: 'rgba(34,197,94,0.3)' },     // green
    { fill: '#f59e0b', glow: 'rgba(245,158,11,0.3)' },    // amber
    { fill: '#a855f7', glow: 'rgba(168,85,247,0.3)' },    // purple
    { fill: '#ec4899', glow: 'rgba(236,72,153,0.3)' },    // pink
    { fill: '#14b8a6', glow: 'rgba(20,184,166,0.3)' },    // teal
  ];
  const grayColor = { fill: '#555', glow: 'rgba(85,85,85,0.15)' };

  // Build adjacency
  const adj = new Map();
  for (const n of graphNodes) adj.set(n.name, new Set());
  for (const e of graphEdges) {
    if (adj.has(e.from)) adj.get(e.from).add(e.to);
    if (adj.has(e.to)) adj.get(e.to).add(e.from);
  }

  // BFS to find components
  const visited = new Set();
  const componentOf = new Map();
  let compIdx = 0;
  for (const n of graphNodes) {
    if (visited.has(n.name)) continue;
    const queue = [n.name];
    visited.add(n.name);
    while (queue.length) {
      const cur = queue.shift();
      componentOf.set(cur, compIdx);
      for (const neighbor of (adj.get(cur) || new Set())) {
        if (!visited.has(neighbor)) { visited.add(neighbor); queue.push(neighbor); }
      }
    }
    compIdx++;
  }

  // Edges
  ctx.lineWidth = 1.5;
  for (const e of graphEdges) {
    if (!visibleNames.has(e.from) || !visibleNames.has(e.to)) continue;
    const from = graphNodes.find(n => n.name === e.from);
    const to = graphNodes.find(n => n.name === e.to);
    if (!from || !to) continue;

    ctx.beginPath();
    ctx.moveTo(from.x, from.y);
    ctx.lineTo(to.x, to.y);
    ctx.strokeStyle = 'rgba(168,85,247,0.2)';
    ctx.stroke();

    const mx = (from.x + to.x) / 2, my = (from.y + to.y) / 2;
    ctx.fillStyle = 'rgba(168,85,247,0.5)';
    ctx.beginPath();
    ctx.arc(mx, my, 3, 0, Math.PI * 2);
    ctx.fill();

    ctx.fillStyle = 'rgba(255,255,255,0.4)';
    ctx.font = '9px sans-serif';
    ctx.fillText(e.relation || '', mx, my - 6);
  }

  // Nodes with relation-family coloring
  for (const n of graphNodes) {
    if (!visibleNames.has(n.name)) continue;
    const comp = componentOf.get(n.name);
    const hasEdges = (adj.get(n.name)?.size || 0) > 0;
    const c = hasEdges ? componentColors[comp % componentColors.length] : grayColor;
    const isSelected = selectedNode === n;

    if (isSelected) {
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.radius + 8, 0, Math.PI * 2);
      ctx.fillStyle = c.glow;
      ctx.fill();
    }

    ctx.beginPath();
    ctx.arc(n.x, n.y, n.radius, 0, Math.PI * 2);
    ctx.fillStyle = c.fill;
    ctx.fill();
    ctx.strokeStyle = isSelected ? '#fff' : c.glow;
    ctx.lineWidth = isSelected ? 2 : 1;
    ctx.stroke();

    ctx.fillStyle = '#e2e8f0';
    ctx.font = 'bold 10px sans-serif';
    ctx.textAlign = 'center';
    ctx.shadowColor = 'rgba(0,0,0,0.8)';
    ctx.shadowBlur = 3;
    ctx.fillText(n.name, n.x, n.y + n.radius + 14);
    ctx.shadowBlur = 0;
  }

  ctx.restore();
}

function selectNode(node) {
  selectedNode = node;
  const detail = document.getElementById('kb-graph-detail');
  if (detail) {
    detail.innerHTML = `
      <div class="kb-graph-detail-name">${node.name}</div>
      <div>Type: <span class="type-badge ${node.type || 'Other'}">${node.type || 'Other'}</span></div>
      <div>Relations: ${(graphEdges || []).filter(e => e.from === node.name || e.to === node.name).map(e => e.relation).join(', ') || 'none'}</div>
    `;
  }
}

/* ── Nodes Tab ───────────────────────────────────────────── */

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
      const edgeCount = (graphEdges || []).filter(e => e.from === n.name || e.to === n.name).length;
      return `<tr>
        <td>${n.name}</td>
        <td><span class="type-badge ${n.type || 'Other'}">${n.type || 'Other'}</span></td>
        <td>${edgeCount}</td>
        <td><button class="kb-delete-btn" data-delete-node="${n.name}">\u2717</button></td>
      </tr>`;
    }).join('') || '<tr><td colspan="4" style="text-align:center;color:var(--text-muted)">No nodes found</td></tr>';

    tbody.querySelectorAll('[data-delete-node]').forEach(btn => {
      btn.addEventListener('click', () => deleteNode(btn.dataset.deleteNode));
    });
  }

  document.getElementById('kb-nodes-search')?.addEventListener('input', renderNodeTable);
  document.getElementById('kb-nodes-filter')?.addEventListener('change', renderNodeTable);
  renderNodeTable();
}

/* ── Edges Tab ──────────────────────────────────────────── */

function renderEdgesTab(content) {
  content.innerHTML = `
    <div style="overflow-y:auto;flex:1">
      <table class="kb-table">
        <thead><tr><th>From</th><th>Relation</th><th>To</th><th>Confidence</th><th>Actions</th></tr></thead>
        <tbody id="kb-edges-body"></tbody>
      </table>
    </div>
  `;

  function renderEdgeTable() {
    const tbody = document.getElementById('kb-edges-body');
    if (!tbody) return;
    const edges = graphEdges || [];

    tbody.innerHTML = edges.map((e, i) => `
      <tr>
        <td>${e.from}</td>
        <td>${e.relation}</td>
        <td>${e.to}</td>
        <td>${(e.confidence || 0).toFixed(2)}</td>
        <td><button class="kb-delete-btn" data-delete-edge="${i}">\u2717</button></td>
      </tr>
    `).join('') || '<tr><td colspan="5" style="text-align:center;color:var(--text-muted)">No edges found</td></tr>';

    tbody.querySelectorAll('[data-delete-edge]').forEach(btn => {
      btn.addEventListener('click', () => deleteEdge(parseInt(btn.dataset.deleteEdge)));
    });
  }

  renderEdgeTable();
}

/* ── Files Tab ───────────────────────────────────────────── */

function renderFilesTab(content) {
  content.innerHTML = '<div style="text-align:center;padding:24px;color:var(--text-muted)">Loading files...</div>';

  if (getIpc()) {
    invoke('get_files').then(result => {
      const files = Array.isArray(result) ? result : [];
      kbData.files = files;
      renderFilesTable(content);
    }).catch(e => {
      console.warn('get_files failed:', e);
      kbData.files = [];
      renderFilesTable(content);
    });
  } else {
    kbData.files = [];
    renderFilesTable(content);
  }
}

function renderFilesTable(content) {
  const files = kbData.files || [];
  content.innerHTML = `
    <div style="overflow-y:auto;flex:1">
      <table class="kb-table">
        <thead><tr><th>File</th><th>Chars</th><th>Source</th><th>Date</th><th>Actions</th></tr></thead>
        <tbody id="kb-files-body"></tbody>
      </table>
    </div>
  `;

  const tbody = document.getElementById('kb-files-body');
  if (!tbody) return;

  tbody.innerHTML = files.length === 0
    ? '<tr><td colspan="5" style="text-align:center;color:var(--text-muted);padding:32px">No files ingested yet. OCR a PDF and approve it to see it here.</td></tr>'
    : files.map(f => `
    <tr>
      <td>\uD83D\uDCC4 ${f.name || 'unknown'}</td>
      <td>${(f.size || 0).toLocaleString()}</td>
      <td>${f.source || '-'}</td>
      <td>${(f.date || '').slice(0, 10)}</td>
      <td>
        ${f.path ? `<button class="kb-open-btn" data-open="${f.path}">\uD83D\uDCC2 Open original</button>` : ''}
        <button class="kb-open-btn" data-view-ocr="${f.name}">\uD83D\uDCC4 View OCR text</button>
        <button class="kb-delete-btn" data-delete-file="${f.name}">\u2717</button>
      </td>
    </tr>
  `).join('');

  tbody.querySelectorAll('[data-open]').forEach(btn => {
    btn.addEventListener('click', () => {
      import('../lib/ipc.js').then(m => m.openPdf(btn.dataset.open));
    });
  });
  tbody.querySelectorAll('[data-view-ocr]').forEach(btn => {
    btn.addEventListener('click', () => {
      const fileName = btn.dataset.viewOcr;
      const ocrPanel = getPanel('ocr');
      if (ocrPanel) {
        ocrPanel.show();
        const body = ocrPanel.getContentEl();
        body.innerHTML = `
          <button class="ocr-back-btn" id="ocr-kb-back-btn">\u2190 Back to OCR upload</button>
          <div style="padding:12px;color:var(--text-secondary);font-size:var(--font-size-sm)">
            <strong>${fileName}</strong> — text stored in knowledge base.<br/><br/>
            Ask Alice about this document to get answers from it.
          </div>
        `;
        document.getElementById('ocr-kb-back-btn').addEventListener('click', () => {
          import('./ocr-panel.js').then(m => m.resetOcrView());
        });
      }
    });
  });
  tbody.querySelectorAll('[data-delete-file]').forEach(btn => {
    btn.addEventListener('click', () => {
      if (confirm('Remove file record?')) {
        kbData.files = kbData.files.filter(f => f.name !== btn.dataset.deleteFile);
        renderFilesTable(content);
      }
    });
  });
}

/* ── CRUD Operations ─────────────────────────────────────── */

function addNodeDialog() {
  const name = prompt('Node name:');
  if (!name) return;
  const type = prompt('Type (Person/Project/Tool/Concept/Document/Other):', 'Concept');
  if (!type) return;

  graphNodes.push({ name, type: type || 'Concept' });
  kbData.nodes = graphNodes;
  selectedTab = 'graph';
  renderTabContent(getPanel('kb'));

  const ipcFn = getIpc();
  if (ipcFn) invoke('kb_add_node', { name, nodeType: type }).catch(() => {});
}

function addEdgeDialog() {
  const from = prompt('From node:');
  if (!from) return;
  const to = prompt('To node:');
  if (!to) return;
  const relation = prompt('Relation (USES/PREFERS/WORKS_ON/...):', 'RELATED_TO');
  if (!relation) return;

  graphEdges.push({ from, to, relation: relation || 'RELATED_TO', confidence: 0.8 });
  kbData.edges = graphEdges;
  selectedTab = 'graph';
  renderTabContent(getPanel('kb'));

  const ipcFn = getIpc();
  if (ipcFn) invoke('kb_add_edge', { from, to, relation, confidence: 0.8 }).catch(() => {});
}

function removeSelectedNode() {
  if (!selectedNode) return;
  if (!confirm('Remove "' + selectedNode.name + '" and all its edges?')) return;

  graphNodes = graphNodes.filter(n => n !== selectedNode);
  graphEdges = graphEdges.filter(e => e.from !== selectedNode.name && e.to !== selectedNode.name);
  kbData.nodes = graphNodes;
  kbData.edges = graphEdges;
  selectedNode = null;
  renderTabContent(getPanel('kb'));
}

function deleteNode(name) {
  if (!confirm('Delete node "' + name + '"?')) return;
  graphNodes = graphNodes.filter(n => n.name !== name);
  graphEdges = graphEdges.filter(e => e.from !== name && e.to !== name);
  kbData.nodes = graphNodes;
  kbData.edges = graphEdges;
  renderTabContent(getPanel('kb'));
}

function deleteEdge(index) {
  if (!confirm('Delete this edge?')) return;
  graphEdges.splice(index, 1);
  kbData.edges = graphEdges;
  renderTabContent(getPanel('kb'));
}

function deleteFile(name) {
  if (!confirm('Delete file "' + name + '" from knowledge base?')) return;
  kbData.files = (kbData.files || []).filter(f => f.name !== name);
  renderTabContent(getPanel('kb'));
}

function formatSize(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(0) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
}
