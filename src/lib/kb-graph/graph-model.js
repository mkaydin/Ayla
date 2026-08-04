const clamp01 = value => Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0));

export function normalizeNodeSources(node) {
  const values = Array.isArray(node?.sources) && node.sources.length
    ? node.sources
    : (node?.source ? [node.source] : []);
  return [...new Set(values.map(value => String(value || '').trim()).filter(Boolean))];
}

function normalizedLog(value, maximum) {
  if (maximum <= 1) return value > 0 ? 1 : 0;
  return Math.log1p(Math.max(0, value)) / Math.log1p(maximum);
}

function calculatePageRank(nodes, edges, adjacency) {
  const count = nodes.length;
  if (!count) return new Map();
  const damping = 0.85;
  let rank = new Map(nodes.map(node => [node.id, 1 / count]));

  for (let iteration = 0; iteration < 24; iteration++) {
    const next = new Map(nodes.map(node => [node.id, (1 - damping) / count]));
    let dangling = 0;
    for (const node of nodes) {
      const links = adjacency.get(node.id) || [];
      const totalWeight = links.reduce((sum, link) => sum + link.weight, 0);
      if (totalWeight <= 0) {
        dangling += rank.get(node.id) || 0;
        continue;
      }
      for (const link of links) {
        next.set(
          link.target,
          (next.get(link.target) || 0)
            + damping * (rank.get(node.id) || 0) * (link.weight / totalWeight),
        );
      }
    }
    if (dangling > 0) {
      const share = damping * dangling / count;
      for (const node of nodes) next.set(node.id, (next.get(node.id) || 0) + share);
    }
    rank = next;
  }
  return rank;
}

/**
 * Convert the persisted KB response into an undirected, weighted layout graph.
 * The result is a projection only: it never changes the persisted graph.
 */
export function buildWeightedGraph(rawNodes = [], rawEdges = []) {
  const nodeById = new Map();
  for (const raw of rawNodes) {
    const id = String(raw?.name || '').trim();
    if (!id || nodeById.has(id)) continue;
    nodeById.set(id, {
      id,
      label: id,
      type: String(raw?.type || 'Other'),
      sources: normalizeNodeSources(raw),
      created: String(raw?.created || ''),
      degree: 0,
      weightedDegree: 0,
      pageRank: 0,
      importance: 0,
      mass: 1,
    });
  }

  const aggregated = new Map();
  for (const raw of rawEdges) {
    const source = String(raw?.from || '').trim();
    const target = String(raw?.to || '').trim();
    if (!source || !target || source === target || !nodeById.has(source) || !nodeById.has(target)) continue;
    const key = source < target ? `${source}\u0000${target}` : `${target}\u0000${source}`;
    let edge = aggregated.get(key);
    if (!edge) {
      edge = {
        id: key,
        source: source < target ? source : target,
        target: source < target ? target : source,
        confidence: 0,
        occurrenceCount: 0,
        relations: new Set(),
        sources: new Set(),
      };
      aggregated.set(key, edge);
    }
    edge.confidence = Math.max(edge.confidence, clamp01(Number(raw?.confidence ?? 0.8)));
    edge.occurrenceCount++;
    if (raw?.relation) edge.relations.add(String(raw.relation));
    for (const sourceId of Array.isArray(raw?.sources) ? raw.sources : []) {
      if (sourceId) edge.sources.add(String(sourceId));
    }
  }

  let maximumEvidence = 1;
  for (const edge of aggregated.values()) {
    const endpointSources = new Set([
      ...nodeById.get(edge.source).sources,
      ...nodeById.get(edge.target).sources,
    ]);
    edge.crossDocument = edge.sources.size > 1 || endpointSources.size > 1;
    edge.evidenceCount = Math.max(1, edge.sources.size, edge.occurrenceCount);
    maximumEvidence = Math.max(maximumEvidence, edge.evidenceCount);
  }

  const edges = [];
  const adjacency = new Map([...nodeById.keys()].map(id => [id, []]));
  for (const edge of aggregated.values()) {
    const confidenceFactor = edge.confidence;
    const evidenceFactor = normalizedLog(edge.evidenceCount, maximumEvidence);
    const crossDocumentFactor = edge.crossDocument ? 1 : 0;
    const weight = Math.max(
      0.05,
      0.45 * confidenceFactor + 0.30 * evidenceFactor + 0.25 * crossDocumentFactor,
    );
    const serializable = {
      id: edge.id,
      source: edge.source,
      target: edge.target,
      confidence: edge.confidence,
      occurrenceCount: edge.occurrenceCount,
      evidenceCount: edge.evidenceCount,
      crossDocument: edge.crossDocument,
      relations: [...edge.relations].sort(),
      sources: [...edge.sources].sort(),
      weight,
    };
    edges.push(serializable);
    adjacency.get(edge.source).push({ target: edge.target, weight });
    adjacency.get(edge.target).push({ target: edge.source, weight });
  }

  const nodes = [...nodeById.values()];
  for (const node of nodes) {
    const links = adjacency.get(node.id) || [];
    node.degree = links.length;
    node.weightedDegree = links.reduce((sum, link) => sum + link.weight, 0);
  }

  const pageRank = calculatePageRank(nodes, edges, adjacency);
  const maximumWeightedDegree = Math.max(1, ...nodes.map(node => node.weightedDegree));
  const maximumDegree = Math.max(1, ...nodes.map(node => node.degree));
  const maximumPageRank = Math.max(1 / Math.max(1, nodes.length), ...pageRank.values());
  const maximumSources = Math.max(1, ...nodes.map(node => node.sources.length));
  for (const node of nodes) {
    node.pageRank = pageRank.get(node.id) || 0;
    const weightedDegree = node.weightedDegree / maximumWeightedDegree;
    const degree = node.degree / maximumDegree;
    const rank = node.pageRank / maximumPageRank;
    const sourceFrequency = normalizedLog(node.sources.length, maximumSources);
    node.importance = clamp01(
      0.45 * weightedDegree
      + 0.25 * rank
      + 0.20 * sourceFrequency
      + 0.10 * degree,
    );
    node.mass = 1
      + 0.35 * Math.log1p(node.weightedDegree)
      + 0.25 * rank
      + 0.20 * Math.log1p(node.sources.length)
      + 0.20 * degree;
  }

  nodes.sort((a, b) => a.id.localeCompare(b.id));
  edges.sort((a, b) => a.id.localeCompare(b.id));
  return {
    nodes,
    edges,
    adjacency: Object.fromEntries(
      [...adjacency.entries()].map(([id, links]) => [id, links]),
    ),
  };
}

