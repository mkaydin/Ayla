// Bounded node and edge tables, independent of canvas layout.
import { escapeHtml, safeCssToken } from '../../lib/dom.js';

const nodeDisplayName = node => String(node?.displayName || node?.name || '');

export function renderNodesTab(content, { graphNodes, degMap, maxRows }) {
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
      <span class="kb-row-count" id="kb-nodes-count"></span>
    </div>
    <div style="overflow-y:auto;flex:1">
      <table class="kb-table">
        <thead><tr><th>Name</th><th>Type</th><th>Edges</th></tr></thead>
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
      if (search && !n.name.toLowerCase().includes(search)
          && !nodeDisplayName(n).toLowerCase().includes(search)) return false;
      return true;
    });
    const visible = filtered.slice(0, maxRows);
    const count = document.getElementById('kb-nodes-count');
    if (count) {
      count.textContent = filtered.length > visible.length
        ? `${visible.length} of ${filtered.length}`
        : `${filtered.length} nodes`;
    }
    tbody.innerHTML = visible.map(n => {
      const type = safeCssToken(n.type);
      return `<tr><td>${escapeHtml(nodeDisplayName(n))}${n.schema === 'v2' ? `<span class="kb-v2-uid">${escapeHtml(n.name)}</span>` : ''}</td><td><span class="type-badge ${type}">${escapeHtml(n.type || 'Other')}</span></td><td>${degMap[n.name] || 0}</td></tr>`;
    }).join('') || '<tr><td colspan="3" style="text-align:center;color:var(--text-muted)">No nodes found</td></tr>';
  }
  document.getElementById('kb-nodes-search')?.addEventListener('input', renderNodeTable);
  document.getElementById('kb-nodes-filter')?.addEventListener('change', renderNodeTable);
  renderNodeTable();
}

/* ════════════════════════════════════════════════════════════
   Edges tab
   ════════════════════════════════════════════════════════════ */

export function renderEdgesTab(content, { graphNodes, graphEdges, maxRows }) {
  content.innerHTML = `
    <div style="overflow-y:auto;flex:1">
      <table class="kb-table">
        <thead><tr><th>From</th><th>Relation</th><th>To</th><th>Conf</th></tr></thead>
        <tbody id="kb-edges-body"></tbody>
      </table>
    </div>
  `;
  const tbody = document.getElementById('kb-edges-body');
  if (!tbody) return;
  const names = new Map(graphNodes.map(node => [node.name, nodeDisplayName(node)]));
  const visibleEdges = graphEdges.slice(0, maxRows);
  tbody.innerHTML = visibleEdges.map(e => `
    <tr><td>${escapeHtml(names.get(e.from) || e.from)}</td><td>${escapeHtml(e.relation)}</td><td>${escapeHtml(names.get(e.to) || e.to)}</td><td>${(e.confidence||0).toFixed(2)}</td></tr>
  `).join('') || '<tr><td colspan="4" style="text-align:center;color:var(--text-muted)">No edges found</td></tr>';
  if (graphEdges.length > visibleEdges.length) {
    const row = document.createElement('tr');
    const cell = document.createElement('td');
    cell.colSpan = 4;
    cell.className = 'kb-table-limit';
    cell.textContent = `Showing ${visibleEdges.length} of ${graphEdges.length} edges`;
    row.appendChild(cell);
    tbody.appendChild(row);
  }
}

/* ════════════════════════════════════════════════════════════
   Graph-v2 structured data and audit history
   ════════════════════════════════════════════════════════════ */

