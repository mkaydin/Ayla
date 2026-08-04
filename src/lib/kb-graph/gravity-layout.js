import { fitClusterCircle } from './cluster-geometry.js';

const CLUSTER_COLORS = [
  '#a78bfa', '#60a5fa', '#34d399', '#f472b6', '#fbbf24',
  '#22d3ee', '#fb7185', '#c084fc', '#4ade80', '#f97316',
];

const clamp = (value, minimum, maximum) => Math.max(minimum, Math.min(maximum, value));
const GOLDEN_ANGLE = Math.PI * (3 - Math.sqrt(5));

function usesCompactCommunityLayout(cluster) {
  const count = cluster.nodeIds.length;
  return count <= 12 || (count <= 48 && cluster.density < 0.45);
}

function hashString(value) {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index++) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function percentile(values, fraction) {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * fraction))];
}

/** Deterministic weighted modularity local-moving pass. */
export function detectWeightedCommunities(graph, maxIterations = 32) {
  const ids = graph.nodes.map(node => node.id);
  const community = new Map(ids.map(id => [id, id]));
  const degree = new Map(graph.nodes.map(node => [node.id, node.weightedDegree]));
  const totalWeight2 = Math.max(1e-9, graph.edges.reduce((sum, edge) => sum + edge.weight, 0) * 2);
  const communityWeight = new Map(ids.map(id => [id, degree.get(id) || 0]));
  const adjacency = new Map(Object.entries(graph.adjacency || {}));
  const order = [...graph.nodes]
    .sort((a, b) => b.weightedDegree - a.weightedDegree || a.id.localeCompare(b.id));

  for (let iteration = 0; iteration < maxIterations; iteration++) {
    let moved = false;
    for (const node of order) {
      const current = community.get(node.id);
      const nodeDegree = degree.get(node.id) || 0;
      const scores = new Map();
      for (const link of adjacency.get(node.id) || []) {
        const candidate = community.get(link.target);
        scores.set(candidate, (scores.get(candidate) || 0) + link.weight);
      }
      if (!scores.size) continue;

      communityWeight.set(current, Math.max(0, (communityWeight.get(current) || 0) - nodeDegree));
      let best = current;
      let bestGain = scores.get(current) || 0;
      bestGain -= nodeDegree * (communityWeight.get(current) || 0) / totalWeight2;
      for (const [candidate, internalWeight] of scores) {
        const gain = internalWeight
          - nodeDegree * (communityWeight.get(candidate) || 0) / totalWeight2;
        if (
          gain > bestGain + 1e-9
          || (Math.abs(gain - bestGain) <= 1e-9 && String(candidate).localeCompare(String(best)) < 0)
        ) {
          best = candidate;
          bestGain = gain;
        }
      }
      community.set(node.id, best);
      communityWeight.set(best, (communityWeight.get(best) || 0) + nodeDegree);
      if (best !== current) moved = true;
    }
    if (!moved) break;
  }

  const groups = new Map();
  for (const node of graph.nodes) {
    const key = community.get(node.id);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(node.id);
  }
  const ordered = [...groups.values()]
    .map(nodeIds => nodeIds.sort())
    .sort((a, b) => b.length - a.length || a[0].localeCompare(b[0]));
  const result = {};
  ordered.forEach((nodeIds, index) => {
    for (const id of nodeIds) result[id] = `community-${index}`;
  });
  return result;
}

function buildClusters(graph, memberships) {
  const clustersById = new Map();
  const nodeById = new Map(graph.nodes.map(node => [node.id, node]));
  for (const node of graph.nodes) {
    const clusterId = memberships[node.id];
    if (!clustersById.has(clusterId)) {
      clustersById.set(clusterId, {
        id: clusterId,
        nodeIds: [],
        sourceIds: new Set(),
        internalWeight: 0,
        externalWeight: 0,
        internalEdges: 0,
        mass: 0,
        density: 0,
        radius: 0,
        role: 'independent',
        parentId: null,
        satellites: [],
        affinities: {},
      });
    }
    const cluster = clustersById.get(clusterId);
    cluster.nodeIds.push(node.id);
    node.sources.forEach(source => cluster.sourceIds.add(source));
    cluster.mass += node.mass;
  }

  const crossWeights = new Map();
  for (const edge of graph.edges) {
    const sourceCluster = memberships[edge.source];
    const targetCluster = memberships[edge.target];
    if (sourceCluster === targetCluster) {
      const cluster = clustersById.get(sourceCluster);
      cluster.internalWeight += edge.weight;
      cluster.internalEdges++;
    } else {
      clustersById.get(sourceCluster).externalWeight += edge.weight;
      clustersById.get(targetCluster).externalWeight += edge.weight;
      const key = sourceCluster < targetCluster
        ? `${sourceCluster}\u0000${targetCluster}`
        : `${targetCluster}\u0000${sourceCluster}`;
      const value = (crossWeights.get(key) || 0) + edge.weight;
      crossWeights.set(key, value);
    }
  }

  const clusters = [...clustersById.values()];
  for (const cluster of clusters) {
    const count = cluster.nodeIds.length;
    const possible = count > 1 ? count * (count - 1) / 2 : 1;
    cluster.density = clamp(cluster.internalEdges / possible, 0, 1);
    cluster.mass += 0.35 * Math.log1p(cluster.internalWeight)
      + 0.25 * Math.log1p(cluster.externalWeight)
      + 0.20 * Math.log1p(cluster.sourceIds.size);
    cluster.radius = 70 + 28 * Math.sqrt(count) + 42 * (1 - cluster.density);
    cluster.sourceIds = [...cluster.sourceIds].sort();
  }

  const maximumConnection = new Map();
  for (const [key, weight] of crossWeights) {
    const [sourceId, targetId] = key.split('\u0000');
    maximumConnection.set(sourceId, Math.max(maximumConnection.get(sourceId) || 0, weight));
    maximumConnection.set(targetId, Math.max(maximumConnection.get(targetId) || 0, weight));
  }
  for (const [key, weight] of crossWeights) {
    const [sourceId, targetId] = key.split('\u0000');
    const source = clustersById.get(sourceId);
    const target = clustersById.get(targetId);
    const sourceSet = new Set(source.sourceIds);
    const sharedSources = target.sourceIds.filter(id => sourceSet.has(id)).length;
    const sourceUnion = new Set([...source.sourceIds, ...target.sourceIds]).size || 1;
    const provenanceAffinity = sharedSources / sourceUnion;
    const sourceShare = weight / Math.max(weight, source.externalWeight);
    const targetShare = weight / Math.max(weight, target.externalWeight);
    const sourcePeak = weight / Math.max(weight, maximumConnection.get(sourceId) || 0);
    const targetPeak = weight / Math.max(weight, maximumConnection.get(targetId) || 0);
    source.affinities[targetId] = {
      affinity: clamp(0.65 * sourceShare + 0.20 * sourcePeak + 0.15 * provenanceAffinity, 0, 1),
      weight,
    };
    target.affinities[sourceId] = {
      affinity: clamp(0.65 * targetShare + 0.20 * targetPeak + 0.15 * provenanceAffinity, 0, 1),
      weight,
    };
  }

  const majorThreshold = percentile(clusters.map(cluster => cluster.mass), 0.70);
  const maximumMajors = clamp(Math.ceil(Math.sqrt(clusters.length)), 1, 12);
  const majors = [...clusters]
    .filter(cluster => cluster.mass >= majorThreshold)
    .sort((a, b) => b.mass - a.mass || a.id.localeCompare(b.id))
    .slice(0, maximumMajors);
  for (const cluster of clusters) {
    if (majors.includes(cluster)) {
      cluster.role = 'major';
      continue;
    }
    const candidates = majors
      .map(major => ({ id: major.id, affinity: cluster.affinities[major.id]?.affinity || 0 }))
      .sort((a, b) => b.affinity - a.affinity || a.id.localeCompare(b.id));
    const best = candidates[0];
    const second = candidates[1];
    if (best && best.affinity >= 0.45 && (!second || best.affinity - second.affinity >= 0.10)) {
      cluster.role = 'satellite';
      cluster.parentId = best.id;
      clustersById.get(best.id)?.satellites.push(cluster.id);
      continue;
    }
    if (candidates.filter(candidate => candidate.affinity >= 0.25).length >= 2) {
      cluster.role = 'bridge';
    }
  }

  clusters.sort((a, b) => b.mass - a.mass || a.id.localeCompare(b.id));
  clusters.forEach((cluster, index) => {
    cluster.color = CLUSTER_COLORS[index % CLUSTER_COLORS.length];
    cluster.nodeIds.sort();
  });
  return { clusters, clustersById, nodeById };
}

function initializeClusterPositions(clusters, width, height) {
  const centerX = width / 2;
  const centerY = height / 2;
  const positions = new Map();
  const majors = clusters.filter(cluster => cluster.role === 'major');
  const majorRadius = Math.max(260, 120 * Math.sqrt(majors.length));
  majors.forEach((cluster, index) => {
    const angle = -Math.PI / 2 + (2 * Math.PI * index) / Math.max(1, majors.length);
    const radial = majors.length === 1 ? 0 : majorRadius;
    positions.set(cluster.id, {
      x: centerX + Math.cos(angle) * radial,
      y: centerY + Math.sin(angle) * radial,
      vx: 0,
      vy: 0,
    });
  });

  const pending = clusters.filter(cluster => cluster.role !== 'major');
  pending.forEach((cluster, index) => {
    const seed = hashString(cluster.id);
    const angle = (seed % 6283) / 1000;
    const parent = cluster.parentId ? positions.get(cluster.parentId) : null;
    const radial = parent
      ? (clusters.find(item => item.id === cluster.parentId)?.radius || 100) + cluster.radius + 100
      : majorRadius + 260 + 80 * Math.sqrt(index);
    positions.set(cluster.id, {
      x: (parent?.x ?? centerX) + Math.cos(angle) * radial,
      y: (parent?.y ?? centerY) + Math.sin(angle) * radial,
      vx: 0,
      vy: 0,
    });
  });
  return positions;
}

function simulateClusters(clusters, positions, width, height) {
  const byId = new Map(clusters.map(cluster => [cluster.id, cluster]));
  const centerX = width / 2;
  const centerY = height / 2;
  for (let iteration = 0; iteration < 180; iteration++) {
    const cooling = 1 - iteration / 200;
    for (let left = 0; left < clusters.length; left++) {
      const a = clusters[left];
      const pa = positions.get(a.id);
      for (let right = left + 1; right < clusters.length; right++) {
        const b = clusters[right];
        const pb = positions.get(b.id);
        let dx = pb.x - pa.x;
        let dy = pb.y - pa.y;
        let distance = Math.hypot(dx, dy);
        if (distance < 0.01) {
          const angle = (hashString(`${a.id}:${b.id}`) % 6283) / 1000;
          dx = Math.cos(angle);
          dy = Math.sin(angle);
          distance = 1;
        }
        const nx = dx / distance;
        const ny = dy / distance;
        const affinity = a.affinities[b.id]?.affinity || 0;
        const minimum = a.radius + b.radius + 70;
        const repulsion = (4200 * (1.15 - affinity)) / Math.max(80, distance);
        pa.vx -= nx * repulsion / Math.max(1, a.mass);
        pa.vy -= ny * repulsion / Math.max(1, a.mass);
        pb.vx += nx * repulsion / Math.max(1, b.mass);
        pb.vy += ny * repulsion / Math.max(1, b.mass);
        if (distance < minimum) {
          const collision = (minimum - distance) * 0.08;
          pa.vx -= nx * collision;
          pa.vy -= ny * collision;
          pb.vx += nx * collision;
          pb.vy += ny * collision;
        }
        if (affinity > 0) {
          const ideal = minimum + 260 * (1 - affinity);
          const spring = clamp((distance - ideal) * 0.008 * affinity, -4, 4);
          pa.vx += nx * spring;
          pa.vy += ny * spring;
          pb.vx -= nx * spring;
          pb.vy -= ny * spring;
        }
      }
    }

    for (const cluster of clusters) {
      const position = positions.get(cluster.id);
      if (cluster.role === 'satellite' && cluster.parentId) {
        const parent = positions.get(cluster.parentId);
        const parentCluster = byId.get(cluster.parentId);
        const dx = position.x - parent.x;
        const dy = position.y - parent.y;
        const distance = Math.max(1, Math.hypot(dx, dy));
        const affinity = cluster.affinities[cluster.parentId]?.affinity || 0;
        const orbit = parentCluster.radius + cluster.radius + 40 + 160 * (1 - affinity);
        const spring = clamp((distance - orbit) * 0.018, -5, 5);
        position.vx -= (dx / distance) * spring;
        position.vy -= (dy / distance) * spring;
      }
      position.vx += (centerX - position.x) * 0.00045;
      position.vy += (centerY - position.y) * 0.00045;
      position.vx *= 0.78;
      position.vy *= 0.78;
      position.x += position.vx * cooling;
      position.y += position.vy * cooling;
    }
  }
}

function layoutClusterNodes(graph, cluster, memberships, clusterPositions, nodePositions) {
  const center = clusterPositions.get(cluster.id);
  const nodeById = new Map(graph.nodes.map(node => [node.id, node]));
  const nodes = cluster.nodeIds.map(id => nodeById.get(id)).filter(Boolean)
    .sort((a, b) => b.importance - a.importance || a.id.localeCompare(b.id));
  const local = new Map();
  nodes.forEach((node, index) => {
    if (index === 0) {
      local.set(node.id, { x: 0, y: 0, vx: 0, vy: 0 });
      return;
    }
    const angle = index * 2.399963229728653 + (hashString(node.id) % 100) / 100;
    const radial = cluster.radius * 0.82 * Math.sqrt(index / Math.max(1, nodes.length - 1));
    local.set(node.id, {
      x: Math.cos(angle) * radial,
      y: Math.sin(angle) * radial,
      vx: 0,
      vy: 0,
    });
  });

  const internalEdges = graph.edges.filter(edge => memberships[edge.source] === cluster.id && memberships[edge.target] === cluster.id);
  const externalTargets = new Map();
  for (const edge of graph.edges) {
    const sourceCluster = memberships[edge.source];
    const targetCluster = memberships[edge.target];
    if (sourceCluster === targetCluster) continue;
    const localId = sourceCluster === cluster.id ? edge.source : (targetCluster === cluster.id ? edge.target : null);
    const remoteCluster = sourceCluster === cluster.id ? targetCluster : sourceCluster;
    if (!localId) continue;
    const target = clusterPositions.get(remoteCluster);
    if (!target) continue;
    const value = externalTargets.get(localId) || { x: 0, y: 0, weight: 0 };
    value.x += (target.x - center.x) * edge.weight;
    value.y += (target.y - center.y) * edge.weight;
    value.weight += edge.weight;
    externalTargets.set(localId, value);
  }

  // Force relaxation can flatten sparse communities into a chain because a
  // handful of edge springs overpower their weak radial force. A compact,
  // regular orbit is clearer for small groups and keeps their community shape
  // visibly circular.
  if (usesCompactCommunityLayout(cluster)) {
    const angleOffset = (hashString(cluster.id) % 6283) / 1000;
    nodes.forEach((node, index) => {
      let angle = angleOffset;
      let orbitRadius = 0;
      if (nodes.length >= 2 && nodes.length <= 8) {
        orbitRadius = clamp(22 + nodes.length * 4.5, 28, cluster.radius * 0.55);
        angle += (Math.PI * 2 * index) / nodes.length;
      } else if (nodes.length > 8 && index > 0) {
        // A golden-angle disk avoids rows, spokes and overlapping rings while
        // keeping sparse medium-sized communities compact.
        angle += index * GOLDEN_ANGLE;
        orbitRadius = 22 * Math.sqrt(index);
      }
      nodePositions[node.id] = {
        x: center.x + Math.cos(angle) * orbitRadius,
        y: center.y + Math.sin(angle) * orbitRadius,
        r: clamp(5 + 10 * node.importance + Math.log1p(node.weightedDegree), 5, 20),
        color: cluster.color,
        clusterId: cluster.id,
        importance: node.importance,
        bridgeScore: externalTargets.has(node.id) ? 1 : 0,
      };
    });
    return;
  }

  for (let iteration = 0; iteration < 90; iteration++) {
    const cooling = 1 - iteration / 100;
    for (const edge of internalEdges) {
      const source = local.get(edge.source);
      const target = local.get(edge.target);
      let dx = target.x - source.x;
      let dy = target.y - source.y;
      const distance = Math.max(1, Math.hypot(dx, dy));
      dx /= distance;
      dy /= distance;
      const ideal = 42 + 72 * (1 - edge.weight);
      const spring = clamp((distance - ideal) * 0.012 * edge.weight, -2.5, 2.5);
      source.vx += dx * spring;
      source.vy += dy * spring;
      target.vx -= dx * spring;
      target.vy -= dy * spring;
    }

    const grid = new Map();
    const cellSize = 34;
    for (const node of nodes) {
      const position = local.get(node.id);
      const gx = Math.floor(position.x / cellSize);
      const gy = Math.floor(position.y / cellSize);
      const key = `${gx},${gy}`;
      if (!grid.has(key)) grid.set(key, []);
      grid.get(key).push(node);
    }
    for (const node of nodes) {
      const position = local.get(node.id);
      const gx = Math.floor(position.x / cellSize);
      const gy = Math.floor(position.y / cellSize);
      for (let x = gx - 1; x <= gx + 1; x++) {
        for (let y = gy - 1; y <= gy + 1; y++) {
          for (const other of grid.get(`${x},${y}`) || []) {
            if (other.id <= node.id) continue;
            const otherPosition = local.get(other.id);
            let dx = otherPosition.x - position.x;
            let dy = otherPosition.y - position.y;
            let distance = Math.hypot(dx, dy);
            if (distance < 0.01) {
              const angle = (hashString(`${node.id}:${other.id}`) % 6283) / 1000;
              dx = Math.cos(angle);
              dy = Math.sin(angle);
              distance = 1;
            }
            const minimum = 18 + 8 * (node.importance + other.importance);
            if (distance < minimum) {
              const push = (minimum - distance) * 0.08;
              position.vx -= dx / distance * push;
              position.vy -= dy / distance * push;
              otherPosition.vx += dx / distance * push;
              otherPosition.vy += dy / distance * push;
            }
          }
        }
      }
    }

    for (const node of nodes) {
      const position = local.get(node.id);
      const distance = Math.max(1, Math.hypot(position.x, position.y));
      const desiredRadius = cluster.radius * (0.20 + 0.68 * (1 - node.importance));
      const radial = (distance - desiredRadius) * 0.005;
      position.vx -= position.x / distance * radial;
      position.vy -= position.y / distance * radial;
      const external = externalTargets.get(node.id);
      if (external?.weight) {
        position.vx += (external.x / external.weight - position.x) * 0.0018;
        position.vy += (external.y / external.weight - position.y) * 0.0018;
      }
      position.vx *= 0.74;
      position.vy *= 0.74;
      position.x += position.vx * cooling;
      position.y += position.vy * cooling;
      const radius = Math.hypot(position.x, position.y);
      const maximumRadius = cluster.radius * 0.94;
      if (radius > maximumRadius) {
        position.x *= maximumRadius / radius;
        position.y *= maximumRadius / radius;
      }
    }
  }

  for (const node of nodes) {
    const position = local.get(node.id);
    nodePositions[node.id] = {
      x: center.x + position.x,
      y: center.y + position.y,
      r: clamp(5 + 10 * node.importance + Math.log1p(node.weightedDegree), 5, 20),
      color: cluster.color,
      clusterId: cluster.id,
      importance: node.importance,
      bridgeScore: externalTargets.has(node.id) ? 1 : 0,
    };
  }
}

function boundsFor(nodePositions, clusters) {
  const values = Object.values(nodePositions);
  if (!values.length) return null;
  return {
    minX: Math.min(...values.map(position => position.x - position.r - 36), ...clusters.map(cluster => cluster.x - cluster.radius - 30)),
    maxX: Math.max(...values.map(position => position.x + position.r + 36), ...clusters.map(cluster => cluster.x + cluster.radius + 30)),
    minY: Math.min(...values.map(position => position.y - position.r - 36), ...clusters.map(cluster => cluster.y - cluster.radius - 30)),
    maxY: Math.max(...values.map(position => position.y + position.r + 36), ...clusters.map(cluster => cluster.y + cluster.radius + 30)),
  };
}

export function calculateGravityLayout(graph, options = {}) {
  const width = Math.max(640, Number(options.width) || 1200);
  const height = Math.max(480, Number(options.height) || 800);
  const memberships = detectWeightedCommunities(graph);
  const { clusters } = buildClusters(graph, memberships);
  const clusterPositions = initializeClusterPositions(clusters, width, height);
  simulateClusters(clusters, clusterPositions, width, height);

  const nodePositions = {};
  for (const cluster of clusters) {
    layoutClusterNodes(graph, cluster, memberships, clusterPositions, nodePositions);
  }

  const previous = options.previousPositions || {};
  if (!options.reset) {
    const compactClusterIds = new Set(
      clusters.filter(usesCompactCommunityLayout).map(item => item.id),
    );
    for (const [id, position] of Object.entries(nodePositions)) {
      const old = previous[id];
      if (!old || !Number.isFinite(old.x) || !Number.isFinite(old.y)) continue;
      // Do not preserve the legacy line shape for compact communities; their
      // regular orbit should take effect immediately.
      const movement = compactClusterIds.has(position.clusterId) ? 1 : 0.25;
      position.x = old.x * (1 - movement) + position.x * movement;
      position.y = old.y * (1 - movement) + position.y * movement;
    }
  }

  const layoutNodeById = new Map(graph.nodes.map(node => [node.id, node]));
  const serializableClusters = clusters.map(cluster => {
    const topNodes = cluster.nodeIds
      .map(id => layoutNodeById.get(id))
      .filter(Boolean)
      .sort((a, b) => b.importance - a.importance || a.id.localeCompare(b.id))
      .slice(0, 3);
    const simulatedPosition = clusterPositions.get(cluster.id);
    const fitted = fitClusterCircle(cluster.nodeIds, nodePositions, {
      padding: 18,
      minimumRadius: 52,
    });
    const position = fitted || simulatedPosition;
    return {
      id: cluster.id,
      label: topNodes[0]?.label || cluster.id,
      topNodeIds: topNodes.map(node => node.id),
      nodeIds: cluster.nodeIds,
      sourceIds: cluster.sourceIds,
      mass: cluster.mass,
      density: cluster.density,
      radius: fitted?.radius || cluster.radius,
      role: cluster.role,
      parentId: cluster.parentId,
      satelliteIds: cluster.satellites,
      color: cluster.color,
      x: position.x,
      y: position.y,
    };
  });

  return {
    positions: nodePositions,
    clusters: serializableClusters,
    memberships,
    edgeWeights: Object.fromEntries(graph.edges.map(edge => [edge.id, edge.weight])),
    bounds: boundsFor(nodePositions, serializableClusters),
  };
}
