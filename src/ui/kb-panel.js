import { renderNodesTab, renderEdgesTab } from './kb/tables.js';
import { renderFilesTab } from './kb/files-tab.js';
// ── Knowledge Base Panel —— optimized Canvas2D graph ────────

import { createPanel } from './panel-system.js';
import {
  hasDesktopBackend,
  invoke,
  listenEvent,
} from '../lib/ipc.js';
import { events } from '../lib/events.js';
import { escapeHtml, safeCssToken } from '../lib/dom.js';
import { selectRenderResolution } from '../lib/render-performance.js';
import { notifyProcess } from '../lib/process-notify.js';
import { fitClusterCircle } from '../lib/kb-graph/cluster-geometry.js';

/* ── State ──────────────────────────────────────────────── */
let kbData      = { nodes: [], edges: [], files: [] };
let graphV2Data = {
  available: false,
  nodes: [],
  edges: [],
  runs: [],
  nodesTruncated: false,
  edgesTruncated: false,
  runsTruncated: false,
};
let selectedTab = 'graph';
let selectedV2View = 'nodes';
let selectedNode = null;
let kbPanelRef  = null;
let graphDataSignature = '';
let cleanupRunning = false;
let cleanupStatus = '';
let graphV2RollbackRunning = false;
let graphV2PageLoading = false;
let graphV2MaintenanceRunning = false;

const GRAPH_LAYOUT_STORAGE_KEY = 'ayla_kb_graph_layout';
const GRAPH_POSITION_STORAGE_KEY = 'ayla_kb_gravity_positions_v1';
const GRAPH_SCHEMA_STORAGE_KEY = 'ayla_kb_graph_schema_v2';
const GRAPH_LAYOUT_CLASSIC = 'classic';
const GRAPH_LAYOUT_GRAVITY = 'gravity';
const GRAPH_COLORS = ['#38bdf8','#22d3ee','#2dd4bf','#60a5fa','#34d399','#67e8f9','#4ade80','#93c5fd','#14b8a6','#7dd3fc'];
const DEFAULT_NODE_COLOR = '#36536b';


const graphFilters = {
  nodeType: 'all',
  source: 'all',
  relation: 'all',
  showWeakRelations: true,
  showProvenance: true,
};

function graphNodeSources(node) {
  return Array.isArray(node?.sources) && node.sources.length
    ? node.sources
    : (node?.source ? [node.source] : []);
}

function graphNodeDisplayName(node) {
  return String(node?.displayName || node?.name || '');
}

function graphV2CanvasData() {
  const nodes = (graphV2Data.nodes || []).map(node => ({
    name: node.uid,
    displayName: node.name,
    type: node.node_type || 'Other',
    label: node.label,
    source: node.sources?.[0] || '',
    sources: node.sources || [],
    aliases: node.aliases || [],
    meaning: node.meaning || '',
    created: node.created || '',
    origins: node.origins || [],
    sourceLocations: node.source_locations || [],
    confidence: node.confidence,
    schema: 'v2',
  }));
  const edges = (graphV2Data.edges || []).map(edge => ({
    from: edge.from_uid,
    to: edge.to_uid,
    relation: edge.relation,
    confidence: edge.confidence,
    sources: edge.sources || [],
    origins: edge.origins || [],
    sourceLocations: edge.source_locations || [],
    uid: edge.uid,
    schema: 'v2',
  }));
  return { nodes, edges };
}

function refreshActiveGraphData() {
  const active = graphV2CanvasData();
  graphNodes = active.nodes;
  graphEdges = active.edges;
  const nextSignature = JSON.stringify({
    schema: 'v2',
    nodes: graphNodes.map(node => [node.name, graphNodeDisplayName(node), node.sources || node.source || '']),
    edges: graphEdges.map(edge => [edge.from, edge.relation, edge.to]),
  });
  if (nextSignature !== graphDataSignature) {
    graphDataSignature = nextSignature;
    layoutDirty[GRAPH_LAYOUT_CLASSIC] = true;
    layoutDirty[GRAPH_LAYOUT_GRAVITY] = true;
    for (const key of Object.keys(layoutPositions[GRAPH_LAYOUT_CLASSIC])) {
      delete layoutPositions[GRAPH_LAYOUT_CLASSIC][key];
    }
    const liveNames = new Set(graphNodes.map(node => node.name));
    for (const key of Object.keys(layoutPositions[GRAPH_LAYOUT_GRAVITY])) {
      if (!liveNames.has(key)) delete layoutPositions[GRAPH_LAYOUT_GRAVITY][key];
    }
  }
  if (selectedNode && !graphNodes.some(node => node.name === selectedNode.name)) {
    selectedNode = null;
    stopSelectedAnimation();
  }
  if (selectedClusterId && !gravityClusters.some(cluster => cluster.id === selectedClusterId)) {
    selectedClusterId = null;
  }
  precompute();
}

function stableColorIndex(value) {
  let hash = 2166136261;
  for (const character of String(value || '')) {
    hash ^= character.codePointAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0) % GRAPH_COLORS.length;
}

function sourceColor(source) {
  return source ? GRAPH_COLORS[stableColorIndex(source)] : DEFAULT_NODE_COLOR;
}

function graphEdgeKey(edge) {
  return edge.from < edge.to
    ? `${edge.from}\u0000${edge.to}`
    : `${edge.to}\u0000${edge.from}`;
}

function graphNodeVisible(node) {
  return (graphFilters.nodeType === 'all' || node.type === graphFilters.nodeType)
    && (graphFilters.source === 'all' || graphNodeSources(node).includes(graphFilters.source));
}

function graphEdgeWeight(edge) {
  return graphLayoutMode === GRAPH_LAYOUT_GRAVITY
    ? Number(gravityEdgeWeights[graphEdgeKey(edge)] || edge.confidence || 0.5)
    : Number(edge.confidence || 0.5);
}

function storedGraphLayoutMode() {
  try {
    return localStorage.getItem(GRAPH_LAYOUT_STORAGE_KEY) === GRAPH_LAYOUT_GRAVITY
      ? GRAPH_LAYOUT_GRAVITY
      : GRAPH_LAYOUT_CLASSIC;
  } catch {
    return GRAPH_LAYOUT_CLASSIC;
  }
}

function storedGravityPositions() {
  try {
    const value = JSON.parse(localStorage.getItem(GRAPH_POSITION_STORAGE_KEY) || '{}');
    return value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  } catch {
    return {};
  }
}

let graphLayoutMode = storedGraphLayoutMode();
const layoutPositions = {
  [GRAPH_LAYOUT_CLASSIC]: {},
  [GRAPH_LAYOUT_GRAVITY]: storedGravityPositions(),
};
const layoutViews = {
  [GRAPH_LAYOUT_CLASSIC]: { ox: 0, oy: 0, zoom: 1 },
  [GRAPH_LAYOUT_GRAVITY]: { ox: 0, oy: 0, zoom: 1 },
};
const layoutDirty = {
  [GRAPH_LAYOUT_CLASSIC]: true,
  [GRAPH_LAYOUT_GRAVITY]: true,
};
let gravityWorker = null;
let gravityRevision = 0;
let gravityClusters = [];
let gravityEdgeWeights = {};
let gravityStatus = '';
let gravityAnimationFrame = 0;
let selectedClusterId = null;

// Graph data
let graphNodes = [];
let graphEdges = [];

// Pre-computed indices (rebuilt ONCE per data change via precompute())
let degMap    = {};     // name → degree
let adjMap    = {};     // name → Set(neighbour names)
let edgeList  = [];     // [{fp, tp}] cached position refs (avoids hashmap lookup in draw)
let spatialGrid = null; // spatial hash for O(1) hit-testing

// Rendering state
let graphCanvas = null;
let graphCtx    = null;
let graphView   = layoutViews[graphLayoutMode];
let panning = false, panSX = 0, panSY = 0;
let panMoved = false, pressedCluster = null;
let dragging = false, dragNode = null;
let hoveredNode = null;
let hoveredCluster = null;
let hoverNeighbours = new Set();
let resizeDebounce = null;
let graphResizeFrame = 0;
let graphResizeObserver = null;
let graphDpr = 1;
let selectedAnimationTimer = 0;
const MAX_NODE_ROWS = 500;
const MAX_EDGE_ROWS = 1000;

// Dirty-flag render: only redraw when something changes, not 60fps idle
let renderPending = false;
function requestRender() {
  if (renderPending || !graphCanvas?.isConnected) return;
  renderPending = true;
  requestAnimationFrame(() => { renderPending = false; draw(); });
}

function scheduleSelectedAnimation() {
  if (!selectedNode || selectedAnimationTimer || !graphCanvas?.isConnected) return;
  if (document.hidden || graphCanvas.offsetParent === null) return;
  if (typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches) return;
  selectedAnimationTimer = window.setTimeout(() => {
    selectedAnimationTimer = 0;
    requestRender();
  }, 42);
}

function stopSelectedAnimation() {
  if (selectedAnimationTimer) window.clearTimeout(selectedAnimationTimer);
  selectedAnimationTimer = 0;
}

function formatCleanupEta(seconds) {
  const value = Math.max(0, Number(seconds) || 0);
  if (value < 60) return `${Math.max(1, Math.round(value))}s`;
  const minutes = Math.ceil(value / 60);
  return minutes < 60 ? `${minutes}m` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

// Node positions
let nodePos = layoutPositions[graphLayoutMode]; // name → { x, y, r, color, clusterId? }

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
  events.on('render-quality-change', () => {
    const container = document.getElementById('kb-graph-container');
    if (container && graphCanvas) {
      fitGraphCanvas(container, graphCanvas);
      requestRender();
    }
  });
  return panel;
}

async function loadKbData() {
  if (hasDesktopBackend()) {
    const structured = await invoke('get_graph_v2_data');
    graphV2Data = structured;
    kbData = { ...graphV2CanvasData(), files: [] };
  }
  refreshActiveGraphData();
}

/* ════════════════════════════════════════════════════════════
   Tab shell
   ════════════════════════════════════════════════════════════ */

function renderKb(panel) {
  const body = panel.getContentEl();
  body.innerHTML = `
    <div class="kb-tabs">
      <div class="kb-tab ${selectedTab === 'graph' ? 'active' : ''}" data-tab="graph">Graph</div>
      <div class="kb-tab ${selectedTab === 'nodes' ? 'active' : ''}" data-tab="nodes">Nodes</div>
      <div class="kb-tab ${selectedTab === 'edges' ? 'active' : ''}" data-tab="edges">Edges</div>
      <div class="kb-tab ${selectedTab === 'structured' ? 'active' : ''}" data-tab="structured">Structured</div>
      <div class="kb-tab ${selectedTab === 'runs' ? 'active' : ''}" data-tab="runs">Runs</div>
      <div class="kb-tab ${selectedTab === 'files' ? 'active' : ''}" data-tab="files">Files</div>
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
  if (selectedTab !== 'graph') {
    graphResizeObserver?.disconnect();
    graphResizeObserver = null;
    graphCanvas = null;
    graphCtx = null;
  }
  switch (selectedTab) {
    case 'graph': renderGraphTab(content, panel); break;
    case 'nodes': renderNodesTab(content, { graphNodes, degMap, maxRows: MAX_NODE_ROWS }); break;
    case 'edges': renderEdgesTab(content, { graphNodes, graphEdges, maxRows: MAX_EDGE_ROWS }); break;
    case 'structured': renderStructuredTab(content); break;
    case 'runs': renderGraphV2RunsTab(content, panel); break;
    case 'files': renderFilesTab(content); break;
  }
}

/* ════════════════════════════════════════════════════════════
   Pre-compute indices — called ONCE per data change
   ════════════════════════════════════════════════════════════ */

function precompute() {
  degMap = {}; adjMap = {};
  for (const n of graphNodes) { degMap[n.name] = 0; adjMap[n.name] = new Set(); }
  for (const e of graphEdges) {
    if (degMap[e.from] != null) { degMap[e.from]++; adjMap[e.from]?.add(e.to); }
    if (degMap[e.to]   != null) { degMap[e.to]++;   adjMap[e.to]?.add(e.from); }
  }
  edgeList = graphEdges
    .map(e => {
      return {
        fp: nodePos[e.from],
        tp: nodePos[e.to],
        edge: e,
        weight: graphEdgeWeight(e),
      };
    })
    .filter(e => e.fp && e.tp);
  buildSpatialIndex();
}

/** Spatial hash grid for O(1) hit-testing. */
function buildSpatialIndex() {
  const cell = 60;
  const grid = new Map();
  for (const n of graphNodes) {
    const p = nodePos[n.name];
    if (!p) continue;
    const key = (Math.floor(p.x / cell)) + ',' + (Math.floor(p.y / cell));
    let arr = grid.get(key);
    if (!arr) { arr = []; grid.set(key, arr); }
    arr.push(n);
  }
  spatialGrid = { grid, cell };
}

function activateGraphLayout(mode) {
  graphLayoutMode = mode === GRAPH_LAYOUT_GRAVITY ? GRAPH_LAYOUT_GRAVITY : GRAPH_LAYOUT_CLASSIC;
  nodePos = layoutPositions[graphLayoutMode];
  graphView = layoutViews[graphLayoutMode];
  try { localStorage.setItem(GRAPH_LAYOUT_STORAGE_KEY, graphLayoutMode); } catch {}
}

function persistGravityPositions() {
  try {
    const compact = Object.fromEntries(
      Object.entries(layoutPositions[GRAPH_LAYOUT_GRAVITY]).map(([id, position]) => [id, {
        x: Math.round(position.x * 10) / 10,
        y: Math.round(position.y * 10) / 10,
        r: Math.round(position.r * 10) / 10,
        color: position.color,
        clusterId: position.clusterId,
      }]),
    );
    localStorage.setItem(GRAPH_POSITION_STORAGE_KEY, JSON.stringify(compact));
  } catch {
    // Position persistence is an optimization; storage quota/private mode must not break the graph.
  }
}

function updateGravityStatus(message) {
  gravityStatus = message;
  const status = document.getElementById('kb-gravity-status');
  if (status) status.textContent = message;
  const overlay = document.getElementById('kb-graph-layout-status');
  if (overlay) {
    overlay.textContent = message;
    overlay.hidden = !message || !message.startsWith('Calculating');
  }
}

function refreshGravityClusterGeometry() {
  if (!gravityClusters.length) return;
  const positions = layoutPositions[GRAPH_LAYOUT_GRAVITY];
  for (const cluster of gravityClusters) {
    const fitted = fitClusterCircle(cluster.nodeIds, positions, {
      padding: 18,
      minimumRadius: 52,
    });
    if (fitted) Object.assign(cluster, fitted);
  }
}

function applyGravityLayoutResult(result, container, { fit = true, timingMs = 0 } = {}) {
  if (gravityAnimationFrame) cancelAnimationFrame(gravityAnimationFrame);
  gravityAnimationFrame = 0;

  const positions = layoutPositions[GRAPH_LAYOUT_GRAVITY];
  const targets = result.positions || {};
  const liveNames = new Set(graphNodes.map(node => node.name));
  for (const key of Object.keys(positions)) if (!liveNames.has(key)) delete positions[key];

  const starts = {};
  for (const [id, target] of Object.entries(targets)) {
    const current = positions[id];
    starts[id] = current && Number.isFinite(current.x) && Number.isFinite(current.y)
      ? { x: current.x, y: current.y }
      : { x: target.x, y: target.y };
    if (!current) positions[id] = { ...target };
    else Object.assign(current, {
      r: target.r,
      color: target.color,
      clusterId: target.clusterId,
      importance: target.importance,
      bridgeScore: target.bridgeScore,
    });
  }

  const clusterTargets = Array.isArray(result.clusters) ? result.clusters : [];
  gravityClusters = clusterTargets.map(target => ({ ...target }));
  refreshGravityClusterGeometry();
  gravityEdgeWeights = result.edgeWeights || {};
  layoutDirty[GRAPH_LAYOUT_GRAVITY] = false;

  if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY && graphCanvas?.isConnected) {
    activateGraphLayout(GRAPH_LAYOUT_GRAVITY);
    precompute();
    updateCount();
    if (fit) fitGraphView(result.bounds, container.clientWidth, container.clientHeight);
  }

  const reduceMotion = typeof matchMedia === 'function'
    && matchMedia('(prefers-reduced-motion: reduce)').matches;
  const duration = reduceMotion ? 0 : 650;
  const startedAt = performance.now();
  updateGravityStatus(`${gravityClusters.length} communities · settling…`);

  const finish = () => {
    for (const [id, target] of Object.entries(targets)) Object.assign(positions[id], target);
    refreshGravityClusterGeometry();
    buildSpatialIndex();
    persistGravityPositions();
    updateGravityStatus(
      `${gravityClusters.length} communities · ${Math.round(Number(timingMs) || 0)} ms`,
    );
    if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY) requestRender();
  };

  if (!duration) {
    finish();
    return;
  }

  const animate = now => {
    const progress = Math.min(1, (now - startedAt) / duration);
    const eased = 1 - Math.pow(1 - progress, 3);
    for (const [id, target] of Object.entries(targets)) {
      const start = starts[id];
      positions[id].x = start.x + (target.x - start.x) * eased;
      positions[id].y = start.y + (target.y - start.y) * eased;
    }
    refreshGravityClusterGeometry();
    if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY) requestRender();
    if (progress < 1) gravityAnimationFrame = requestAnimationFrame(animate);
    else {
      gravityAnimationFrame = 0;
      finish();
    }
  };
  gravityAnimationFrame = requestAnimationFrame(animate);
}

function requestGravityLayout(container, { reset = false, fit = true } = {}) {
  if (!container || !graphNodes.length) return;
  gravityWorker?.terminate();
  gravityWorker = null;
  const revision = ++gravityRevision;
  updateGravityStatus('Calculating relationship layout…');

  try {
    gravityWorker = new Worker(
      new URL('../workers/kb-gravity.worker.js', import.meta.url),
      { type: 'module', name: 'ayla-kb-gravity-layout' },
    );
  } catch (error) {
    updateGravityStatus(`Relationship layout unavailable: ${error?.message || error}`);
    return;
  }

  gravityWorker.onmessage = event => {
    if (revision !== gravityRevision) return;
    const message = event.data || {};
    if (message.type === 'layout-error') {
      updateGravityStatus(`Layout failed: ${message.error || 'unknown error'}`);
      gravityWorker?.terminate();
      gravityWorker = null;
      return;
    }
    if (message.type !== 'layout-result') return;

    applyGravityLayoutResult(message.result || {}, container, {
      fit,
      timingMs: message.timingMs,
    });
    gravityWorker?.terminate();
    gravityWorker = null;
  };
  gravityWorker.onerror = event => {
    if (revision !== gravityRevision) return;
    updateGravityStatus(`Layout worker failed: ${event.message || 'unknown error'}`);
    gravityWorker?.terminate();
    gravityWorker = null;
  };
  gravityWorker.postMessage({
    revision,
    nodes: graphNodes,
    edges: graphEdges,
    options: {
      width: Math.max(900, container.clientWidth * 2),
      height: Math.max(700, container.clientHeight * 2),
      previousPositions: reset ? {} : layoutPositions[GRAPH_LAYOUT_GRAVITY],
      reset,
    },
  });
}

/* ════════════════════════════════════════════════════════════
   Graph tab
   ════════════════════════════════════════════════════════════ */

function renderGraphTab(content, panel) {
  activateGraphLayout(graphLayoutMode);
  const showingV2 = graphV2Data.available;
  const sourceValues = [...new Set(graphNodes.flatMap(graphNodeSources))]
    .sort((a, b) => a.localeCompare(b));
  const relationValues = [...new Set(graphEdges.map(edge => edge.relation).filter(Boolean))]
    .sort((a, b) => a.localeCompare(b));
  if (graphFilters.source !== 'all' && !sourceValues.includes(graphFilters.source)) graphFilters.source = 'all';
  if (graphFilters.relation !== 'all' && !relationValues.includes(graphFilters.relation)) graphFilters.relation = 'all';
  const sourceOptions = sourceValues
    .map(source => `<option value="${escapeHtml(source)}" ${graphFilters.source === source ? 'selected' : ''}>${escapeHtml(source)}</option>`)
    .join('');
  const relationOptions = relationValues
    .map(relation => `<option value="${escapeHtml(relation)}" ${graphFilters.relation === relation ? 'selected' : ''}>${escapeHtml(relation)}</option>`)
    .join('');
  content.innerHTML = `
    <div class="kb-graph-container" id="kb-graph-container">
      <canvas id="kb-graph-canvas"></canvas>
      <div class="kb-graph-layout-status" id="kb-graph-layout-status" hidden></div>
    </div>
    <div class="kb-graph-controls">
      <span class="kb-graph-view-label">Graph v2 (UID)</span>
      <label class="kb-graph-view-label" for="kb-graph-view">View:</label>
      <select class="kb-select" id="kb-graph-view">
        <option value="${GRAPH_LAYOUT_CLASSIC}" ${graphLayoutMode === GRAPH_LAYOUT_CLASSIC ? 'selected' : ''}>Document Clusters</option>
        <option value="${GRAPH_LAYOUT_GRAVITY}" ${graphLayoutMode === GRAPH_LAYOUT_GRAVITY ? 'selected' : ''}>Relationship Gravity</option>
      </select>
      <input class="kb-search-input" id="kb-graph-search" placeholder="Search node..." style="width:140px" />
      <span>Type:</span>
      <select class="kb-select" id="kb-graph-filter">
        <option value="all" ${graphFilters.nodeType === 'all' ? 'selected' : ''}>All</option>
        <option value="Person" ${graphFilters.nodeType === 'Person' ? 'selected' : ''}>Person</option>
        <option value="Project" ${graphFilters.nodeType === 'Project' ? 'selected' : ''}>Project</option>
        <option value="Tool" ${graphFilters.nodeType === 'Tool' ? 'selected' : ''}>Tool</option>
        <option value="Concept" ${graphFilters.nodeType === 'Concept' ? 'selected' : ''}>Concept</option>
        <option value="Document" ${graphFilters.nodeType === 'Document' ? 'selected' : ''}>Document</option>
      </select>
      <select class="kb-select kb-graph-source-filter" id="kb-graph-source" title="Filter by source document">
        <option value="all">All sources</option>
        ${sourceOptions}
      </select>
      <select class="kb-select kb-graph-relation-filter" id="kb-graph-relation" title="Filter relations">
        <option value="all">All relations</option>
        ${relationOptions}
      </select>
      <label class="kb-graph-toggle" title="Hide relations with calculated weight below 0.5">
        <input type="checkbox" id="kb-graph-weak" ${graphFilters.showWeakRelations ? 'checked' : ''} /> Weak edges
      </label>
      <label class="kb-graph-toggle" title="Show segmented source rings on multi-document nodes">
        <input type="checkbox" id="kb-graph-provenance" ${graphFilters.showProvenance ? 'checked' : ''} /> Provenance
      </label>
      <button class="kb-graph-btn" id="kb-graph-layout">Re-layout</button>
      ${showingV2 && (graphV2Data.nodesTruncated || graphV2Data.edgesTruncated)
        ? `<button class="kb-graph-btn" id="kb-graph-load-more" ${graphV2PageLoading ? 'disabled' : ''}>${graphV2PageLoading ? 'Loading…' : 'Load more v2'}</button>`
        : ''}
      <button class="kb-graph-btn" id="kb-graph-reset-focus" ${selectedClusterId || selectedNode ? '' : 'disabled'}>Reset focus</button>
      <button class="kb-graph-btn accent" id="kb-graph-cleanup" ${cleanupRunning || !hasDesktopBackend() ? 'disabled' : ''} title="Improve shared conversation and document concepts">
        ${cleanupRunning ? 'Analyzing…' : 'AI Analyze & Improve'}
      </button>
      <span class="kb-cleanup-status" id="kb-cleanup-status">${escapeHtml(cleanupStatus)}</span>
      <span class="kb-gravity-status" id="kb-gravity-status">${escapeHtml(graphLayoutMode === GRAPH_LAYOUT_GRAVITY ? gravityStatus : '')}</span>
      <span id="kb-graph-count" style="margin-left:auto;color:var(--text-muted)"></span>
    </div>
    <div class="kb-graph-detail" id="kb-graph-detail">
      <div style="color:var(--text-muted)">Click to select · Scroll to zoom · Drag node to move · Drag background to pan</div>
    </div>
  `;

  const canvas = document.getElementById('kb-graph-canvas');
  const container = document.getElementById('kb-graph-container');
  if (!canvas || !container) return;
  graphCanvas = canvas;
  graphCtx = canvas.getContext('2d');

  // Clear stale positions for deleted nodes, then create an immediate fallback
  // layout. Relationship Gravity replaces it asynchronously from a worker.
  const liveNames = new Set(graphNodes.map(node => node.name));
  for (const key of Object.keys(nodePos)) if (!liveNames.has(key)) delete nodePos[key];
  const hadGravityPositions = graphLayoutMode === GRAPH_LAYOUT_GRAVITY
    && Object.keys(nodePos).length > 0;
  const missingPositions = graphNodes.some(node => !nodePos[node.name]);
  if (graphLayoutMode === GRAPH_LAYOUT_CLASSIC && (layoutDirty[GRAPH_LAYOUT_CLASSIC] || missingPositions)) {
    const bounds = runClassicLayout(container.clientWidth, container.clientHeight);
    fitGraphView(bounds, container.clientWidth, container.clientHeight);
    layoutDirty[GRAPH_LAYOUT_CLASSIC] = false;
  } else if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY && missingPositions) {
    const bounds = runClassicLayout(container.clientWidth, container.clientHeight);
    fitGraphView(bounds, container.clientWidth, container.clientHeight);
  }
  precompute();
  updateCount();

  // Resize — update canvas size immediately, debounce the re-layout
  function fitNow() {
    fitGraphCanvas(container, canvas);
    requestRender();
  }
  graphResizeObserver?.disconnect();
  graphResizeObserver = new ResizeObserver(() => {
    if (!graphResizeFrame) {
      graphResizeFrame = requestAnimationFrame(() => {
        graphResizeFrame = 0;
        fitNow();
      });
    }
    if (resizeDebounce) clearTimeout(resizeDebounce);
    resizeDebounce = setTimeout(() => {
      precompute();
      requestRender();
    }, 180);
  });
  graphResizeObserver.observe(container);
  fitNow();
  if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY && layoutDirty[GRAPH_LAYOUT_GRAVITY]) {
    requestGravityLayout(container, {
      reset: !hadGravityPositions,
      fit: true,
    });
  }

  // ── Interactions ──────────────────────────────────────
  canvas.addEventListener('wheel', (e) => {
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    const cx = mx / graphView.zoom - graphView.ox;
    const cy = my / graphView.zoom - graphView.oy;
    graphView.zoom = Math.max(0.1, Math.min(8, graphView.zoom * (e.deltaY < 0 ? 1.12 : 1/1.12)));
    graphView.ox = mx / graphView.zoom - cx;
    graphView.oy = my / graphView.zoom - cy;
    requestRender();
  });

  canvas.addEventListener('pointerdown', (e) => {
    const rect = canvas.getBoundingClientRect();
    const mx = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
    const my = (e.clientY - rect.top)  / graphView.zoom - graphView.oy;
    const hit = hitTest(mx, my);
    if (hit) {
      dragging = true; dragNode = hit;
      pressedCluster = null;
      selectNode(hit);
      canvas.setPointerCapture(e.pointerId);
    } else {
      panning = true;
      panMoved = false;
      pressedCluster = graphLayoutMode === GRAPH_LAYOUT_GRAVITY ? hitTestCluster(mx, my) : null;
      panSX = e.clientX;
      panSY = e.clientY;
    }
  });

  canvas.addEventListener('pointermove', (e) => {
    const rect = canvas.getBoundingClientRect();
    if (dragging && dragNode) {
      const p = nodePos[dragNode.name];
      if (p) {
        p.x = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
        p.y = (e.clientY - rect.top)  / graphView.zoom - graphView.oy;
        if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY) refreshGravityClusterGeometry();
        buildSpatialIndex();
        requestRender();
      }
    } else if (panning) {
      const deltaX = e.clientX - panSX;
      const deltaY = e.clientY - panSY;
      if (Math.abs(deltaX) + Math.abs(deltaY) > 2) panMoved = true;
      graphView.ox += deltaX / graphView.zoom;
      graphView.oy += deltaY / graphView.zoom;
      panSX = e.clientX; panSY = e.clientY;
      requestRender();
    } else {
      const mx = (e.clientX - rect.left) / graphView.zoom - graphView.ox;
      const my = (e.clientY - rect.top)  / graphView.zoom - graphView.oy;
      const hit = hitTest(mx, my);
      const clusterHit = !hit && graphLayoutMode === GRAPH_LAYOUT_GRAVITY
        ? hitTestCluster(mx, my)
        : null;
      if (hit !== hoveredNode || clusterHit !== hoveredCluster) {
        hoveredNode = hit;
        hoveredCluster = clusterHit;
        hoverNeighbours = hit ? new Set([hit.name, ...(adjMap[hit.name] || [])]) : new Set();
        canvas.style.cursor = hit || clusterHit ? 'pointer' : 'grab';
        requestRender();
      }
    }
  });

  canvas.addEventListener('pointerup', () => {
    if (panning && !panMoved && pressedCluster) focusCluster(pressedCluster, container);
    if (dragging && dragNode && graphLayoutMode === GRAPH_LAYOUT_GRAVITY) persistGravityPositions();
    dragging = false;
    dragNode = null;
    panning = false;
    panMoved = false;
    pressedCluster = null;
  });
  canvas.addEventListener('pointerleave', () => {
    dragging = false;
    dragNode = null;
    panning = false;
    panMoved = false;
    pressedCluster = null;
    hoveredNode = null;
    hoveredCluster = null;
    hoverNeighbours = new Set();
    requestRender();
  });

  document.getElementById('kb-graph-search')?.addEventListener('input', (e) => {
    const q = e.target.value.toLowerCase();
    if (!q) { requestRender(); return; }
    const hit = graphNodes.find(n =>
      n.name.toLowerCase().includes(q) || graphNodeDisplayName(n).toLowerCase().includes(q));
    if (hit && nodePos[hit.name]) {
      const p = nodePos[hit.name];
      graphView.ox = canvas.clientWidth / 2 / graphView.zoom - p.x;
      graphView.oy = canvas.clientHeight / 2 / graphView.zoom - p.y;
      selectNode(hit);
      requestRender();
    }
  });
  document.getElementById('kb-graph-view')?.addEventListener('change', event => {
    activateGraphLayout(event.target.value);
    renderTabContent(panel);
  });
  document.getElementById('kb-graph-filter')?.addEventListener('change', event => {
    graphFilters.nodeType = event.target.value;
    updateCount();
    requestRender();
  });
  document.getElementById('kb-graph-source')?.addEventListener('change', event => {
    graphFilters.source = event.target.value;
    updateCount();
    requestRender();
  });
  document.getElementById('kb-graph-relation')?.addEventListener('change', event => {
    graphFilters.relation = event.target.value;
    updateCount();
    requestRender();
  });
  document.getElementById('kb-graph-weak')?.addEventListener('change', event => {
    graphFilters.showWeakRelations = event.target.checked;
    updateCount();
    requestRender();
  });
  document.getElementById('kb-graph-provenance')?.addEventListener('change', event => {
    graphFilters.showProvenance = event.target.checked;
    requestRender();
  });
  document.getElementById('kb-graph-reset-focus')?.addEventListener('click', () => {
    selectedClusterId = null;
    selectedNode = null;
    stopSelectedAnimation();
    const detail = document.getElementById('kb-graph-detail');
    if (detail) detail.innerHTML = '<div style="color:var(--text-muted)">Click to select · Scroll to zoom · Drag node to move · Drag background to pan</div>';
    const bounds = graphBoundsForActivePositions();
    fitGraphView(bounds, container.clientWidth, container.clientHeight);
    const button = document.getElementById('kb-graph-reset-focus');
    if (button) button.disabled = true;
    requestRender();
  });
  document.getElementById('kb-graph-layout')?.addEventListener('click', () => {
    layoutDirty[graphLayoutMode] = true;
    if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY) {
      requestGravityLayout(container, { reset: true, fit: true });
    } else {
      for (const key of Object.keys(nodePos)) delete nodePos[key];
      const bounds = runClassicLayout(container.clientWidth, container.clientHeight);
      fitGraphView(bounds, container.clientWidth, container.clientHeight);
      layoutDirty[GRAPH_LAYOUT_CLASSIC] = false;
      precompute();
      requestRender();
    }
  });
  document.getElementById('kb-graph-load-more')?.addEventListener('click', async () => {
    try {
      if (graphV2Data.nodesTruncated) await loadMoreGraphV2('nodes');
      if (graphV2Data.edgesTruncated) await loadMoreGraphV2('edges');
    } catch (error) {
      alert(`Graph-v2 page failed: ${String(error)}`);
    }
    renderTabContent(panel);
  });
  document.getElementById('kb-graph-cleanup')?.addEventListener('click', async () => {
    if (!confirm('Run incremental graph improvement? Existing good meanings and relations will be skipped; only candidate duplicates and unresolved items are sent to the LLM. Only high-confidence changes will be applied.')) return;
    const operationId = crypto.randomUUID();
    cleanupRunning = true;
    cleanupStatus = 'Planning incremental analysis…';
    const button = document.getElementById('kb-graph-cleanup');
    const status = document.getElementById('kb-cleanup-status');
    if (button) { button.disabled = true; button.textContent = 'Analyzing…'; }
    if (status) status.textContent = cleanupStatus;
    notifyProcess('start', 'graph-analysis', { operationId });
    let unlistenProgress = () => {};
    try {
      unlistenProgress = await listenEvent('kb-cleanup-progress', event => {
        const progress = event?.payload || event || {};
        if (progress.step === 'planned') {
          const resumed = Number(progress.resumed_batches || 0);
          const local = Number(progress.local_relations || 0);
          cleanupStatus = `${progress.total_batches || 0} AI batches`
            + `${resumed ? ` · ${resumed} resumed` : ''}`
            + `${local ? ` · ${local} local relation fixes` : ''}`
            + `${progress.concurrency === 2 ? ' · 2× cloud' : ''}`;
        } else if (progress.step === 'batch') {
          cleanupStatus = `Batch ${progress.current_batch || 0}/${progress.total_batches || 0}`
            + `${progress.remaining_batches ? ` · ${progress.remaining_batches} left · ETA ${formatCleanupEta(progress.eta_seconds)}` : ' · finishing'}`
            + `${progress.resumed_batches ? ` · ${progress.resumed_batches} resumed` : ''}`;
        } else if (progress.step === 'refining') {
          cleanupStatus = 'Applying high-confidence improvements…';
        } else if (progress.step === 'complete') {
          cleanupStatus = 'Saving improved graph…';
        }
        const progressStatus = document.getElementById('kb-cleanup-status');
        if (progressStatus) progressStatus.textContent = cleanupStatus;
      });
      const result = await invoke('analyze_and_merge_kb');
      const improvements = [
        result.merged_nodes ? `${result.merged_nodes} merged` : '',
        result.renamed_nodes ? `${result.renamed_nodes} renamed` : '',
        result.updated_meanings ? `${result.updated_meanings} meanings` : '',
        result.updated_node_types ? `${result.updated_node_types} types` : '',
        result.relabeled_relations ? `${result.relabeled_relations} relations` : '',
      ].filter(Boolean);
      cleanupStatus = improvements.length
        ? improvements.join(' · ')
        : `No high-confidence improvements found${result.skipped_groups ? ` · ${result.skipped_groups} skipped` : ''}`;
      events.emit('status-log', { type: 'info', message: `KB analysis: ${cleanupStatus}` });
      notifyProcess('complete', 'graph-analysis', {
        operationId,
        mergedNodes: result.merged_nodes || 0,
        renamedNodes: result.renamed_nodes || 0,
        updatedMeanings: result.updated_meanings || 0,
        updatedNodeTypes: result.updated_node_types || 0,
        relabeledRelations: result.relabeled_relations || 0,
        deduplicatedEdges: result.deduplicated_edges || 0,
        skippedGroups: result.skipped_groups || 0,
      });
      await loadKbData();
      renderTabContent(panel);
    } catch (error) {
      cleanupStatus = `Cleanup failed: ${String(error)}`;
      events.emit('status-log', { type: 'error', message: cleanupStatus });
      notifyProcess('error', 'graph-analysis', {
        operationId,
        reason: String(error),
      });
      if (status) status.textContent = cleanupStatus;
    } finally {
      try { unlistenProgress(); } catch {}
      cleanupRunning = false;
      const currentButton = document.getElementById('kb-graph-cleanup');
      if (currentButton) { currentButton.disabled = false; currentButton.textContent = 'AI Analyze & Improve'; }
    }
  });

  requestRender();
}

function fitGraphCanvas(container, canvas) {
  const width = Math.max(1, container.clientWidth);
  const height = Math.max(1, container.clientHeight);
  graphDpr = selectRenderResolution({
    width,
    height,
    devicePixelRatio: window.devicePixelRatio,
    quality: window.__app?.renderQuality || 'auto',
    resizing: window.__app?.isResizing || false,
  });
  const renderWidth = Math.round(width * graphDpr);
  const renderHeight = Math.round(height * graphDpr);
  if (canvas.width !== renderWidth || canvas.height !== renderHeight) {
    canvas.width = renderWidth;
    canvas.height = renderHeight;
  }
}

function updateCount() {
  const el = document.getElementById('kb-graph-count');
  if (!el) return;
  const visibleNodes = new Set(graphNodes.filter(graphNodeVisible).map(node => node.name));
  const visibleEdges = graphEdges.filter(edge => (
    visibleNodes.has(edge.from)
    && visibleNodes.has(edge.to)
    && (graphFilters.relation === 'all' || edge.relation === graphFilters.relation)
    && (graphFilters.showWeakRelations || graphEdgeWeight(edge) >= 0.5)
  ));
  const filtered = visibleNodes.size !== graphNodes.length || visibleEdges.length !== graphEdges.length;
  el.textContent = filtered
    ? `${visibleNodes.size}/${graphNodes.length} nodes · ${visibleEdges.length}/${graphEdges.length} edges`
    : `${graphNodes.length} nodes · ${graphEdges.length} edges`;
}

/* ════════════════════════════════════════════════════════════
   Layout — packed document/source clusters + relation components
   ════════════════════════════════════════════════════════════ */

function runClassicLayout(w, h) {
  if (!graphNodes.length) return null;
  const COLORS = GRAPH_COLORS;
  const DEFAULT_COLOR = DEFAULT_NODE_COLOR;

  // Colour and cluster by document provenance. Shared canonical nodes stay in
  // their first source cluster and their cross-document edges visibly bridge it.
  const sourceColor = {};
  let sci = 0;
  const nodeSources = node => Array.isArray(node.sources) && node.sources.length
    ? node.sources
    : (node.source ? [node.source] : []);
  const sources = [...new Set(graphNodes.flatMap(node => nodeSources(node)))].sort();
  for (const src of sources) { sourceColor[src] = src === '_none' ? DEFAULT_COLOR : COLORS[sci++ % COLORS.length]; }

  const deg = {};
  const nodeByName = new Map(graphNodes.map(node => [node.name, node]));
  for (const n of graphNodes) deg[n.name] = 0;
  for (const e of graphEdges) { deg[e.from] = (deg[e.from]||0)+1; deg[e.to] = (deg[e.to]||0)+1; }

  const clusters = new Map();
  const unassigned = [];
  for (const node of graphNodes) {
    const memberships = nodeSources(node);
    const source = memberships[0] || (node.type === 'Document' ? node.name : '');
    if (source) {
      if (!clusters.has(source)) clusters.set(source, { key: source, source, nodes: [] });
      clusters.get(source).nodes.push(node);
    } else {
      unassigned.push(node);
    }
  }

  // Nodes without document provenance are split by connected component, so
  // newly ingested relation clusters never pile onto an existing cluster.
  const unassignedNames = new Set(unassigned.map(node => node.name));
  const seen = new Set();
  let componentIndex = 0;
  for (const start of unassigned) {
    if (seen.has(start.name)) continue;
    const stack = [start.name];
    const component = [];
    while (stack.length) {
      const name = stack.pop();
      if (seen.has(name) || !unassignedNames.has(name)) continue;
      seen.add(name);
      const node = nodeByName.get(name);
      if (node) component.push(node);
      for (const neighbour of adjMap[name] || []) {
        if (!seen.has(neighbour) && unassignedNames.has(neighbour)) stack.push(neighbour);
      }
    }
    const key = `_component_${componentIndex++}`;
    clusters.set(key, { key, source: '', nodes: component });
  }

  const packed = [...clusters.values()]
    .filter(cluster => cluster.nodes.length)
    .sort((a, b) => a.key.localeCompare(b.key))
    .map(cluster => ({
      ...cluster,
      radius: Math.max(115, 48 * Math.sqrt(cluster.nodes.length)),
    }));
  const aspect = Math.max(0.5, w / Math.max(h, 1));
  const columns = Math.max(1, Math.ceil(Math.sqrt(packed.length * aspect)));
  const rows = Math.ceil(packed.length / columns);
  const maxRadius = Math.max(115, ...packed.map(cluster => cluster.radius));
  const spacing = maxRadius * 2 + 180;
  const originX = w / 2 - ((Math.min(columns, packed.length) - 1) * spacing) / 2;
  const originY = h / 2 - ((rows - 1) * spacing) / 2;

  for (let clusterIndex = 0; clusterIndex < packed.length; clusterIndex++) {
    const cluster = packed[clusterIndex];
    const column = clusterIndex % columns;
    const row = Math.floor(clusterIndex / columns);
    const centerX = originX + column * spacing;
    const centerY = originY + row * spacing;
    const sorted = [...cluster.nodes].sort((a, b) => {
      if (a.type === 'Document' && b.type !== 'Document') return -1;
      if (b.type === 'Document' && a.type !== 'Document') return 1;
      return (deg[b.name] || 0) - (deg[a.name] || 0) || a.name.localeCompare(b.name);
    });
    let cursor = 0;
    if (sorted.length) {
      const hub = sorted[cursor++];
      const radius = 5 + Math.log2((deg[hub.name] || 0) + 1) * 2.2;
      const source = nodeSources(hub)[0] || cluster.source;
      nodePos[hub.name] = {
        x: centerX, y: centerY,
        r: Math.max(7, Math.min(20, radius)),
        color: sourceColor[source] || DEFAULT_COLOR,
      };
    }
    let ring = 1;
    while (cursor < sorted.length) {
      const ringRadius = ring * 64;
      const capacity = Math.max(6, Math.floor((2 * Math.PI * ringRadius) / 54));
      const count = Math.min(capacity, sorted.length - cursor);
      for (let index = 0; index < count; index++) {
        const node = sorted[cursor++];
        const angle = (2 * Math.PI * index) / count - Math.PI / 2 + clusterIndex * 0.37;
        const radius = 5 + Math.log2((deg[node.name] || 0) + 1) * 2.2;
        const source = nodeSources(node)[0] || cluster.source;
        nodePos[node.name] = {
          x: centerX + Math.cos(angle) * ringRadius,
          y: centerY + Math.sin(angle) * ringRadius,
          r: Math.max(5, Math.min(20, radius)),
          color: sourceColor[source] || DEFAULT_COLOR,
        };
      }
      ring++;
    }
  }

  const positions = Object.values(nodePos);
  if (!positions.length) return null;
  return {
    minX: Math.min(...positions.map(position => position.x - position.r - 24)),
    maxX: Math.max(...positions.map(position => position.x + position.r + 24)),
    minY: Math.min(...positions.map(position => position.y - position.r - 24)),
    maxY: Math.max(...positions.map(position => position.y + position.r + 24)),
  };
}

function fitGraphView(bounds, width, height) {
  if (!bounds) return;
  const graphWidth = Math.max(1, bounds.maxX - bounds.minX);
  const graphHeight = Math.max(1, bounds.maxY - bounds.minY);
  graphView.zoom = Math.max(0.1, Math.min(1, 0.88 * Math.min(width / graphWidth, height / graphHeight)));
  const centerX = (bounds.minX + bounds.maxX) / 2;
  const centerY = (bounds.minY + bounds.maxY) / 2;
  graphView.ox = width / (2 * graphView.zoom) - centerX;
  graphView.oy = height / (2 * graphView.zoom) - centerY;
}

function graphBoundsForActivePositions(nodeIds = null) {
  const allowed = nodeIds ? new Set(nodeIds) : null;
  const positions = graphNodes
    .filter(node => !allowed || allowed.has(node.name))
    .map(node => nodePos[node.name])
    .filter(Boolean);
  if (!positions.length) return null;
  return {
    minX: Math.min(...positions.map(position => position.x - position.r - 36)),
    maxX: Math.max(...positions.map(position => position.x + position.r + 36)),
    minY: Math.min(...positions.map(position => position.y - position.r - 36)),
    maxY: Math.max(...positions.map(position => position.y + position.r + 36)),
  };
}

/* ════════════════════════════════════════════════════════════
   Hit test — O(1) via spatial hash grid
   ════════════════════════════════════════════════════════════ */

function hitTest(mx, my) {
  if (!spatialGrid) return null;
  const { grid, cell } = spatialGrid;
  const gx = Math.floor(mx / cell), gy = Math.floor(my / cell);
  for (let x = gx - 1; x <= gx + 1; x++) {
    for (let y = gy - 1; y <= gy + 1; y++) {
      for (const n of grid.get(`${x},${y}`) || []) {
        const p = nodePos[n.name];
        if (!p) continue;
        const dx = mx - p.x, dy = my - p.y;
        if (dx*dx + dy*dy < (p.r + 4) * (p.r + 4)) return n;
      }
    }
  }
  return null;
}

function hitTestCluster(mx, my) {
  let best = null;
  for (const cluster of gravityClusters) {
    const dx = mx - cluster.x;
    const dy = my - cluster.y;
    if (dx * dx + dy * dy > cluster.radius * cluster.radius) continue;
    if (!best || cluster.radius < best.radius) best = cluster;
  }
  return best;
}

function focusCluster(cluster, container) {
  if (!cluster || !container) return;
  selectedClusterId = cluster.id;
  selectedNode = null;
  stopSelectedAnimation();
  const bounds = graphBoundsForActivePositions(cluster.nodeIds);
  fitGraphView(bounds, container.clientWidth, container.clientHeight);
  const detail = document.getElementById('kb-graph-detail');
  if (detail) {
    const names = new Map(graphNodes.map(node => [node.name, graphNodeDisplayName(node)]));
    detail.innerHTML = `
      <div class="kb-graph-detail-name">${escapeHtml(cluster.label || 'Knowledge community')}</div>
      <div>Role: ${escapeHtml(cluster.role)}</div>
      <div>Nodes: ${cluster.nodeIds.length} · Sources: ${cluster.sourceIds.length}</div>
      <div>Central concepts: ${escapeHtml((cluster.topNodeIds || []).map(id => names.get(id) || id).join(', ') || 'none')}</div>
    `;
  }
  const button = document.getElementById('kb-graph-reset-focus');
  if (button) button.disabled = false;
  requestRender();
}

function selectNode(node) {
  selectedNode = node;
  selectedClusterId = nodePos[node.name]?.clusterId || null;
  const detail = document.getElementById('kb-graph-detail');
  if (detail && node) {
    const edges = graphEdges.filter(e => e.from === node.name || e.to === node.name);
    const nodeType = safeCssToken(node.type);
    detail.innerHTML = `
      <div class="kb-graph-detail-name">${escapeHtml(graphNodeDisplayName(node))}</div>
      <div>Type: <span class="type-badge ${nodeType}">${escapeHtml(node.type || 'Other')}</span></div>
      <div>Degree: ${edges.length}</div>
      <div>Sources: ${escapeHtml((node.sources || (node.source ? [node.source] : [])).join(', ') || 'none')}</div>
      ${node.schema === 'v2' ? `<div>UID: <span class="kb-v2-inline-uid">${escapeHtml(node.name)}</span></div>` : ''}
      ${node.schema === 'v2' && node.origins?.length ? `<div>Origins: ${escapeHtml(node.origins.join(', '))}</div>` : ''}
      ${node.schema === 'v2' && node.sourceLocations?.length ? `<div>Locations: ${escapeHtml(node.sourceLocations.join(', '))}</div>` : ''}
      ${node.meaning ? `<div class="kb-node-meaning">${escapeHtml(node.meaning)}</div>` : ''}
      ${Array.isArray(node.aliases) && node.aliases.length
        ? `<div>Aliases: ${escapeHtml(node.aliases.join(', '))}</div>`
        : ''}
      ${graphLayoutMode === GRAPH_LAYOUT_GRAVITY && nodePos[node.name]
        ? `<div>Importance: ${Math.round((nodePos[node.name].importance || 0) * 100)}% · Community: ${escapeHtml(nodePos[node.name].clusterId || 'none')}</div>`
        : ''}
    `;
  }
  const button = document.getElementById('kb-graph-reset-focus');
  if (button) button.disabled = false;
  requestRender();
  scheduleSelectedAnimation();
}

/* ════════════════════════════════════════════════════════════
   Render — dirty-flag driven, with culling + LOD + batching
   ════════════════════════════════════════════════════════════ */

function traceHighlightedRelation(ctx, from, to) {
  const dx = to.x - from.x;
  const dy = to.y - from.y;
  const length = Math.sqrt(dx * dx + dy * dy) || 1;
  const middleX = (from.x + to.x) / 2;
  const middleY = (from.y + to.y) / 2;
  ctx.moveTo(from.x, from.y);
  ctx.quadraticCurveTo(
    middleX + (-dy / length) * 12,
    middleY + (dx / length) * 12,
    to.x,
    to.y,
  );
}

function drawSelectedRelationGlow(ctx, visibleNodeNames, vp) {
  if (!selectedNode) return;
  const selectedName = selectedNode.name;
  const phase = performance.now() / 230;
  ctx.save();
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  ctx.lineWidth = 2.2;
  ctx.strokeStyle = 'rgba(112, 232, 255, 0.92)';
  ctx.shadowColor = 'rgba(76, 214, 255, 0.95)';
  ctx.shadowBlur = 10;
  ctx.setLineDash([10, 7]);
  ctx.lineDashOffset = -phase * 8;
  for (const { fp, tp, edge, weight } of edgeList) {
    if (!fp || !tp || (edge.from !== selectedName && edge.to !== selectedName)) continue;
    if (!visibleNodeNames.has(edge.from) || !visibleNodeNames.has(edge.to)) continue;
    if (graphFilters.relation !== 'all' && edge.relation !== graphFilters.relation) continue;
    if (!graphFilters.showWeakRelations && weight < 0.5) continue;
    if (fp.x < vp.l && tp.x < vp.l) continue;
    if (fp.x > vp.r && tp.x > vp.r) continue;
    if (fp.y < vp.t && tp.y < vp.t) continue;
    if (fp.y > vp.b && tp.y > vp.b) continue;
    ctx.beginPath();
    traceHighlightedRelation(ctx, fp, tp);
    ctx.stroke();
  }
  ctx.restore();
}

function draw() {
  const ctx = graphCtx, canvas = graphCanvas;
  if (!ctx || !canvas) return;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  const zoom = graphView.zoom;

  // Background
  ctx.setTransform(graphDpr, 0, 0, graphDpr, 0, 0);
  ctx.fillStyle = '#06101c';
  ctx.fillRect(0, 0, w, h);

  // World-space viewport bounds for culling
  const vp = {
    l: -graphView.ox - 50,
    t: -graphView.oy - 50,
    r: -graphView.ox + w / zoom + 50,
    b: -graphView.oy + h / zoom + 50,
  };

  const hoveredName = hoveredNode?.name;
  const visibleNodeNames = new Set(graphNodes.filter(graphNodeVisible).map(node => node.name));
  const selectedNeighbours = selectedNode
    ? new Set([selectedNode.name, ...(adjMap[selectedNode.name] || [])])
    : null;
  const selectedCluster = graphLayoutMode === GRAPH_LAYOUT_GRAVITY && selectedClusterId
    ? gravityClusters.find(cluster => cluster.id === selectedClusterId)
    : null;
  const focusNames = selectedNeighbours
    || (hoveredName ? hoverNeighbours : (selectedCluster ? new Set(selectedCluster.nodeIds) : null));
  const hasFocus = !!focusNames?.size;

  ctx.save();
  ctx.translate(graphView.ox * zoom, graphView.oy * zoom);
  ctx.scale(zoom, zoom);

  // ── COMMUNITY HALOS — relationship view only ─────────
  if (graphLayoutMode === GRAPH_LAYOUT_GRAVITY && gravityClusters.length) {
    ctx.save();
    for (const cluster of gravityClusters) {
      if (cluster.x + cluster.radius < vp.l || cluster.x - cluster.radius > vp.r) continue;
      if (cluster.y + cluster.radius < vp.t || cluster.y - cluster.radius > vp.b) continue;
      const isSelected = cluster.id === selectedClusterId;
      const isHovered = cluster.id === hoveredCluster?.id;
      const dimmed = selectedClusterId && !isSelected;
      ctx.globalAlpha = dimmed ? 0.012 : (isSelected || isHovered ? 0.13 : (cluster.role === 'major' ? 0.075 : 0.04));
      ctx.fillStyle = cluster.color;
      ctx.beginPath();
      ctx.arc(cluster.x, cluster.y, cluster.radius, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = dimmed ? 0.05 : (isSelected || isHovered ? 0.48 : (cluster.role === 'major' ? 0.24 : 0.14));
      ctx.strokeStyle = cluster.color;
      ctx.lineWidth = cluster.role === 'major' ? 1.5 : 1;
      ctx.stroke();
    }
    ctx.restore();

    ctx.save();
    ctx.font = `600 ${Math.max(10, 12 / Math.max(0.55, zoom))}px sans-serif`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    for (const cluster of gravityClusters) {
      const show = cluster.role === 'major'
        || cluster.id === selectedClusterId
        || cluster.id === hoveredCluster?.id
        || zoom > 0.65;
      if (!show) continue;
      if (cluster.x < vp.l || cluster.x > vp.r || cluster.y < vp.t || cluster.y > vp.b) continue;
      ctx.globalAlpha = selectedClusterId && cluster.id !== selectedClusterId ? 0.18 : 0.72;
      ctx.fillStyle = '#bae6fd';
      const label = String(cluster.label || 'Community');
      ctx.fillText(label.length > 32 ? `${label.slice(0, 29)}…` : label, cluster.x, cluster.y - cluster.radius - 14);
    }
    ctx.restore();
  }

  // ── EDGES — batched into weight buckets and viewport-culled ──
  if (zoom > 0.2) {
    const buckets = graphLayoutMode === GRAPH_LAYOUT_GRAVITY
      ? [
          { minimum: 0, maximum: 0.5, color: 'rgba(132,124,154,0.12)', width: 0.7 },
          { minimum: 0.5, maximum: 0.8, color: 'rgba(150,140,180,0.22)', width: 1.1 },
          { minimum: 0.8, maximum: Infinity, color: 'rgba(180,168,218,0.34)', width: 1.7 },
        ]
      : [{ minimum: 0, maximum: Infinity, color: 'rgba(150,140,170,0.22)', width: zoom > 0.6 ? 1.2 : 0.8 }];
    for (const bucket of buckets) {
      ctx.strokeStyle = bucket.color;
      ctx.lineWidth = bucket.width;
      ctx.beginPath();
      for (const { fp, tp, edge, weight } of edgeList) {
        if (!fp || !tp || weight < bucket.minimum || weight >= bucket.maximum) continue;
        if (!visibleNodeNames.has(edge.from) || !visibleNodeNames.has(edge.to)) continue;
        if (graphFilters.relation !== 'all' && edge.relation !== graphFilters.relation) continue;
        if (!graphFilters.showWeakRelations && weight < 0.5) continue;
        if (selectedNode && edge.from !== selectedNode.name && edge.to !== selectedNode.name) continue;
        if (!selectedNode && hasFocus && !focusNames.has(edge.from) && !focusNames.has(edge.to)) continue;
        if (fp.x < vp.l && tp.x < vp.l) continue;
        if (fp.x > vp.r && tp.x > vp.r) continue;
        if (fp.y < vp.t && tp.y < vp.t) continue;
        if (fp.y > vp.b && tp.y > vp.b) continue;
        if (zoom > 0.5) {
          const mx = (fp.x + tp.x) / 2, my = (fp.y + tp.y) / 2;
          const dx = tp.x - fp.x, dy = tp.y - fp.y;
          const len = Math.sqrt(dx*dx + dy*dy) || 1;
          ctx.moveTo(fp.x, fp.y);
          ctx.quadraticCurveTo(mx + (-dy/len)*12, my + (dx/len)*12, tp.x, tp.y);
        } else {
          ctx.moveTo(fp.x, fp.y);
          ctx.lineTo(tp.x, tp.y);
        }
      }
      ctx.stroke();
    }
  }

  // A selected node animates only its direct relations. This overlay is kept
  // separate from the batched base edges so idle rendering remains unchanged.
  drawSelectedRelationGlow(ctx, visibleNodeNames, vp);

  // ── NODES — batched by colour (1 fillStyle per group), culled ──
  const byColor = new Map();
  for (const n of graphNodes) {
    const p = nodePos[n.name];
    if (!p) continue;
    if (!visibleNodeNames.has(n.name)) continue;
    if (p.x < vp.l || p.x > vp.r || p.y < vp.t || p.y > vp.b) continue;

    let color = p.color;
    let r = p.r;
    if (hasFocus && !focusNames.has(n.name)) {
      color = 'rgba(90,80,96,0.2)';
      r *= 0.7;
    }
    let arr = byColor.get(color);
    if (!arr) { arr = []; byColor.set(color, arr); }
    arr.push({ p, n, r });
  }

  for (const [color, items] of byColor) {
    ctx.fillStyle = color;
    ctx.beginPath();
    for (const { p, r } of items) {
      ctx.moveTo(p.x + r, p.y);
      ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
    }
    ctx.fill();
  }

  // Multi-document evidence is shown as a segmented ring only at useful zoom.
  if (graphFilters.showProvenance && zoom > 0.55) {
    ctx.lineWidth = 1.8;
    for (const node of graphNodes) {
      if (!visibleNodeNames.has(node.name)) continue;
      const sources = graphNodeSources(node);
      if (sources.length < 2) continue;
      const position = nodePos[node.name];
      if (!position || position.x < vp.l || position.x > vp.r || position.y < vp.t || position.y > vp.b) continue;
      const displaySources = sources.slice(0, 8);
      const segment = 2 * Math.PI / displaySources.length;
      displaySources.forEach((source, index) => {
        ctx.beginPath();
        ctx.strokeStyle = sourceColor(source);
        ctx.arc(
          position.x,
          position.y,
          position.r + 3,
          -Math.PI / 2 + segment * index + 0.035,
          -Math.PI / 2 + segment * (index + 1) - 0.035,
        );
        ctx.stroke();
      });
    }
  }

  // Direct neighbours use a cool glow, while the selected node uses a warm
  // glow, making relation endpoints immediately distinguishable.
  if (selectedNode) {
    const neighbours = adjMap[selectedNode.name] || new Set();
    const pulse = 0.78 + Math.sin(performance.now() / 260) * 0.16;
    ctx.save();
    for (const name of neighbours) {
      if (!visibleNodeNames.has(name)) continue;
      const position = nodePos[name];
      if (!position) continue;
      ctx.beginPath();
      ctx.arc(position.x, position.y, position.r + 5, 0, Math.PI * 2);
      ctx.strokeStyle = `rgba(81, 224, 255, ${pulse})`;
      ctx.lineWidth = 2.2;
      ctx.shadowColor = '#42d9ff';
      ctx.shadowBlur = 12;
      ctx.stroke();
    }
    const selectedPosition = nodePos[selectedNode.name];
    if (selectedPosition) {
      ctx.beginPath();
      ctx.arc(selectedPosition.x, selectedPosition.y, selectedPosition.r + 7, 0, Math.PI * 2);
      ctx.strokeStyle = `rgba(255, 190, 92, ${Math.min(1, pulse + 0.12)})`;
      ctx.lineWidth = 3;
      ctx.shadowColor = '#ffad42';
      ctx.shadowBlur = 16;
      ctx.stroke();
    }
    ctx.restore();
  }

  // ── HOVER / SELECT HALO ──────────────────────────────
  if (hoveredNode || selectedNode) {
    const halo = (node, fill, stroke, lw) => {
      const p = nodePos[node.name];
      if (!p) return;
      ctx.beginPath();
      ctx.arc(p.x, p.y, p.r * 1.8, 0, Math.PI * 2);
      ctx.fillStyle = fill; ctx.fill();
      ctx.strokeStyle = stroke; ctx.lineWidth = lw; ctx.stroke();
    };
    if (hoveredNode) halo(hoveredNode, 'rgba(255,255,255,0.06)', 'rgba(255,255,255,0.4)', 1);
    if (selectedNode && selectedNode !== hoveredNode) halo(selectedNode, 'rgba(255,174,70,0.08)', 'rgba(255,196,112,0.5)', 1.5);
  }

  // ── LABELS — LOD: only at sufficient zoom, hubs or hovered only ──
  if (zoom > 0.4) {
    const fontSize = Math.min(11, Math.max(9, 10 * zoom));
    ctx.font = `bold ${fontSize}px sans-serif`;
    ctx.textAlign = 'center';
    const showAll = zoom > 1.5;
    const showHubs = zoom > 0.7;
    for (const n of graphNodes) {
      const p = nodePos[n.name];
      if (!p) continue;
      if (!visibleNodeNames.has(n.name)) continue;
      if (p.x < vp.l || p.x > vp.r || p.y < vp.t || p.y > vp.b) continue;

      const isHovered = n.name === hoveredName;
      if (!showAll && !isHovered && !(showHubs && (degMap[n.name]||0) >= 3)) continue;
      if (hasFocus && !focusNames.has(n.name)) continue;

      ctx.fillStyle = isHovered ? '#ffffff' : '#c8c4d0';
      ctx.fillText(graphNodeDisplayName(n), p.x, p.y + p.r + 12);
    }
  }

  ctx.restore();
  scheduleSelectedAnimation();
}

/* ════════════════════════════════════════════════════════════
   Nodes tab
   ════════════════════════════════════════════════════════════ */

function graphV2Values(records, field) {
  return [...new Set(records.flatMap(record => Array.isArray(record?.[field]) ? record[field] : []))]
    .filter(Boolean)
    .sort((a, b) => String(a).localeCompare(String(b)));
}

function graphV2Option(value) {
  return `<option value="${escapeHtml(value)}">${escapeHtml(value)}</option>`;
}

function mergeGraphV2Records(existing, incoming, key) {
  const identity = record => record[key]
    || `${record.backup_id || ''}:${record.committed_at || ''}:${record.source_id || ''}:${record.action || ''}`;
  const merged = new Map(existing.map(record => [identity(record), record]));
  incoming.forEach(record => merged.set(identity(record), record));
  return [...merged.values()];
}

async function loadMoreGraphV2(kind) {
  if (graphV2PageLoading) return;
  const field = kind;
  const key = kind === 'runs' ? 'event_id' : 'uid';
  const limit = kind === 'nodes' ? 500 : kind === 'edges' ? 1000 : 100;
  const current = Array.isArray(graphV2Data[field]) ? graphV2Data[field] : [];
  graphV2PageLoading = true;
  try {
    const page = await invoke('get_graph_v2_page', {
      kind,
      offset: current.length,
      limit,
    });
    graphV2Data[field] = mergeGraphV2Records(current, page?.[field] || [], key);
    graphV2Data[`${kind}Truncated`] = Boolean(page?.hasMore);
    refreshActiveGraphData();
  } finally {
    graphV2PageLoading = false;
  }
}

function renderStructuredTab(content) {
  if (!graphV2Data.available) {
    content.innerHTML = '<div class="kb-v2-empty">Graph-v2 storage is unavailable. Start Ayla with its database to view structured OCR and code-analysis knowledge.</div>';
    return;
  }
  const nodes = Array.isArray(graphV2Data.nodes) ? graphV2Data.nodes : [];
  const edges = Array.isArray(graphV2Data.edges) ? graphV2Data.edges : [];
  const origins = graphV2Values([...nodes, ...edges], 'origins');
  const sources = graphV2Values([...nodes, ...edges], 'sources');
  const labels = [...new Set(nodes.map(node => node.label).filter(Boolean))]
    .sort((a, b) => String(a).localeCompare(String(b)));
  content.innerHTML = `
    <div class="kb-v2-summary">
      <span><strong>${nodes.length.toLocaleString()}</strong> entities${graphV2Data.nodesTruncated ? '+' : ''}</span>
      <span><strong>${edges.length.toLocaleString()}</strong> relations${graphV2Data.edgesTruncated ? '+' : ''}</span>
      <span>UID-keyed schema v2</span>
    </div>
    <div class="kb-v2-view-toggle">
      <button class="kb-graph-btn ${selectedV2View === 'nodes' ? 'accent' : ''}" data-v2-view="nodes">Entities</button>
      <button class="kb-graph-btn ${selectedV2View === 'edges' ? 'accent' : ''}" data-v2-view="edges">Relations</button>
    </div>
    <div class="kb-table-toolbar kb-v2-filters">
      <input class="kb-search-input" id="kb-v2-search" placeholder="Search structured knowledge..." />
      <select class="kb-select" id="kb-v2-label">
        <option value="all">All labels</option>${labels.map(graphV2Option).join('')}
      </select>
      <select class="kb-select" id="kb-v2-origin">
        <option value="all">All origins</option>${origins.map(graphV2Option).join('')}
      </select>
      <select class="kb-select" id="kb-v2-source">
        <option value="all">All sources</option>${sources.map(graphV2Option).join('')}
      </select>
      <span class="kb-row-count" id="kb-v2-count"></span>
    </div>
    <div class="kb-v2-table-wrap"><table class="kb-table">
      <thead id="kb-v2-head"></thead><tbody id="kb-v2-body"></tbody>
    </table></div>
    ${(selectedV2View === 'nodes' ? graphV2Data.nodesTruncated : graphV2Data.edgesTruncated)
      ? `<button class="kb-graph-btn kb-v2-load-more" id="kb-v2-load-more" ${graphV2PageLoading ? 'disabled' : ''}>${graphV2PageLoading ? 'Loading…' : `Load more ${selectedV2View === 'nodes' ? 'entities' : 'relations'}`}</button>`
      : ''}
  `;
  content.querySelectorAll('[data-v2-view]').forEach(button => {
    button.addEventListener('click', () => {
      selectedV2View = button.dataset.v2View;
      renderStructuredTab(content);
    });
  });
  const renderRows = () => {
    const search = String(document.getElementById('kb-v2-search')?.value || '').toLowerCase();
    const label = document.getElementById('kb-v2-label')?.value || 'all';
    const origin = document.getElementById('kb-v2-origin')?.value || 'all';
    const source = document.getElementById('kb-v2-source')?.value || 'all';
    const uidNames = new Map(nodes.map(node => [node.uid, node.name]));
    const records = selectedV2View === 'nodes' ? nodes : edges;
    const filtered = records.filter(record => {
      if (selectedV2View === 'nodes' && label !== 'all' && record.label !== label) return false;
      if (origin !== 'all' && !(record.origins || []).includes(origin)) return false;
      if (source !== 'all' && !(record.sources || []).includes(source)) return false;
      if (!search) return true;
      const searchable = selectedV2View === 'nodes'
        ? [record.uid, record.name, record.label, record.meaning, ...(record.source_locations || [])]
        : [record.uid, record.relation, uidNames.get(record.from_uid), uidNames.get(record.to_uid), ...(record.source_locations || [])];
      return searchable.some(value => String(value || '').toLowerCase().includes(search));
    });
    const visible = filtered.slice(0, selectedV2View === 'nodes' ? MAX_NODE_ROWS : MAX_EDGE_ROWS);
    const count = document.getElementById('kb-v2-count');
    if (count) count.textContent = filtered.length > visible.length
      ? `${visible.length} of ${filtered.length}`
      : `${filtered.length} ${selectedV2View === 'nodes' ? 'entities' : 'relations'}`;
    const head = document.getElementById('kb-v2-head');
    const body = document.getElementById('kb-v2-body');
    if (!head || !body) return;
    if (selectedV2View === 'nodes') {
      head.innerHTML = '<tr><th>Entity</th><th>Label</th><th>Origin</th><th>Source</th><th>Confidence</th></tr>';
      body.innerHTML = visible.map(node => `<tr title="${escapeHtml((node.source_locations || []).join(', '))}">
        <td><span class="kb-v2-primary">${escapeHtml(node.name)}</span><span class="kb-v2-uid">${escapeHtml(node.uid)}</span></td>
        <td>${escapeHtml(node.label || 'Other')}</td>
        <td>${escapeHtml((node.origins || []).join(', ') || '-')}</td>
        <td>${escapeHtml((node.sources || []).join(', ') || '-')}</td>
        <td>${Number(node.confidence || 0).toFixed(2)}</td>
      </tr>`).join('') || '<tr><td colspan="5" class="kb-table-limit">No structured entities found</td></tr>';
    } else {
      head.innerHTML = '<tr><th>From</th><th>Relation</th><th>To</th><th>Origin</th><th>Source</th></tr>';
      body.innerHTML = visible.map(edge => `<tr title="${escapeHtml((edge.source_locations || []).join(', '))}">
        <td>${escapeHtml(uidNames.get(edge.from_uid) || edge.from_uid)}</td>
        <td>${escapeHtml(edge.relation)}</td>
        <td>${escapeHtml(uidNames.get(edge.to_uid) || edge.to_uid)}</td>
        <td>${escapeHtml((edge.origins || []).join(', ') || '-')}</td>
        <td>${escapeHtml((edge.sources || []).join(', ') || '-')}</td>
      </tr>`).join('') || '<tr><td colspan="5" class="kb-table-limit">No structured relations found</td></tr>';
    }
  };
  ['kb-v2-search', 'kb-v2-label', 'kb-v2-origin', 'kb-v2-source'].forEach(id => {
    document.getElementById(id)?.addEventListener(id === 'kb-v2-search' ? 'input' : 'change', renderRows);
  });
  document.getElementById('kb-v2-load-more')?.addEventListener('click', async () => {
    try {
      await loadMoreGraphV2(selectedV2View);
      renderStructuredTab(content);
    } catch (error) {
      alert(`Graph-v2 page failed: ${String(error)}`);
      renderStructuredTab(content);
    }
  });
  renderRows();
}

function graphRunTitle(run) {
  if (run.action === 'rollback') return `Restored ${String(run.target_backup_id || '').slice(0, 8)}`;
  if (run.action === 'legacy_sync') return 'Legacy graph synchronization';
  if (String(run.source_id || '').startsWith('ocr:')) return 'OCR document';
  if (String(run.source_id || '').startsWith('code-analysis:')) return 'Code analysis';
  return run.action === 'remove' ? 'Source removal' : 'Structured import';
}

function renderGraphV2RunsTab(content, panel) {
  if (!graphV2Data.available) {
    content.innerHTML = '<div class="kb-v2-empty">Graph-v2 run history is unavailable while the database is offline.</div>';
    return;
  }
  const runs = Array.isArray(graphV2Data.runs) ? graphV2Data.runs : [];
  content.innerHTML = `
    <div class="kb-v2-summary">
      <span><strong>${runs.length}</strong> committed operations${graphV2Data.runsTruncated ? '+' : ''}</span>
      <span>Every restore creates a recovery backup first</span>
      <button class="kb-graph-btn" id="kb-v2-prune" ${graphV2MaintenanceRunning || !runs.length ? 'disabled' : ''}>${graphV2MaintenanceRunning ? 'Pruning…' : 'Keep newest 25 backups'}</button>
    </div>
    <div class="kb-v2-table-wrap"><table class="kb-table">
      <thead><tr><th>Operation</th><th>Source</th><th>Affected</th><th>Snapshot</th><th>Committed</th><th>Recovery</th></tr></thead>
      <tbody>${runs.map(run => {
        const backup = String(run.backup_id || '');
        const canRestore = run.backup_available !== false && /^[a-f0-9]{32}$/i.test(backup);
        return `<tr>
          <td><span class="kb-v2-primary">${escapeHtml(graphRunTitle(run))}</span><span class="kb-v2-uid">${escapeHtml(run.action || 'import')}</span></td>
          <td title="${escapeHtml(run.source_id || '')}">${escapeHtml(run.source_id || '-')}</td>
          <td>${Number(run.affected_node_count || 0)} entities / ${Number(run.affected_edge_count || 0)} relations</td>
          <td>${Number(run.node_count || 0)} / ${Number(run.edge_count || 0)}</td>
          <td>${escapeHtml(String(run.committed_at || '').replace('T', ' ').slice(0, 19) || '-')}</td>
          <td>${canRestore ? `<button class="kb-graph-btn" data-v2-restore="${backup}" ${graphV2RollbackRunning ? 'disabled' : ''}>Restore before</button>` : '-'}</td>
        </tr>`;
      }).join('') || '<tr><td colspan="6" class="kb-table-limit">No Graph-v2 operations yet</td></tr>'}</tbody>
    </table></div>
    ${graphV2Data.runsTruncated ? `<button class="kb-graph-btn kb-v2-load-more" id="kb-v2-load-runs" ${graphV2PageLoading ? 'disabled' : ''}>${graphV2PageLoading ? 'Loading…' : 'Load older runs'}</button>` : ''}
  `;
  content.querySelectorAll('[data-v2-restore]').forEach(button => {
    button.addEventListener('click', async () => {
      const backupId = button.dataset.v2Restore;
      if (graphV2RollbackRunning || !confirm('Restore graph data from before this operation? The current graph is backed up first. Document and vector approval states remain in effect.')) return;
      graphV2RollbackRunning = true;
      renderGraphV2RunsTab(content, panel);
      try {
        await invoke('rollback_graph_v2', { backupId });
        await loadKbData();
        graphV2RollbackRunning = false;
        renderKb(panel);
      } catch (error) {
        graphV2RollbackRunning = false;
        renderGraphV2RunsTab(content, panel);
        alert(`Graph-v2 restore failed: ${String(error)}`);
        return;
      }
    });
  });
  document.getElementById('kb-v2-load-runs')?.addEventListener('click', async () => {
    try {
      await loadMoreGraphV2('runs');
    } catch (error) {
      alert(`Graph-v2 run page failed: ${String(error)}`);
    }
    renderGraphV2RunsTab(content, panel);
  });
  document.getElementById('kb-v2-prune')?.addEventListener('click', async () => {
    if (graphV2MaintenanceRunning
        || !confirm('Keep the newest 25 Graph-v2 recovery backups and expire older backup tables? Audit history will remain.')) return;
    graphV2MaintenanceRunning = true;
    renderGraphV2RunsTab(content, panel);
    try {
      const result = await invoke('prune_graph_v2_backups', { retention: 25 });
      await loadKbData();
      alert(`Graph-v2 backup maintenance complete: ${Number(result?.retained || 0)} retained, ${Number(result?.pruned || 0)} pruned.`);
    } catch (error) {
      alert(`Graph-v2 backup maintenance failed: ${String(error)}`);
    } finally {
      graphV2MaintenanceRunning = false;
      renderKb(panel);
    }
  });
}

/* ════════════════════════════════════════════════════════════
   Files tab
   ════════════════════════════════════════════════════════════ */
