import test from 'node:test';
import assert from 'node:assert/strict';

import { buildWeightedGraph } from '../src/lib/kb-graph/graph-model.js';
import { fitClusterCircle } from '../src/lib/kb-graph/cluster-geometry.js';
import {
  calculateGravityLayout,
  detectWeightedCommunities,
} from '../src/lib/kb-graph/gravity-layout.js';

function node(name, source) {
  return { name, type: 'Concept', sources: [source], created: '2026-01-01T00:00:00Z' };
}

function edge(from, to, confidence = 0.9, sources = ['evidence']) {
  return { from, to, relation: 'related_to', confidence, sources };
}

test('weighted graph aggregates parallel relations and rewards cross-document evidence', () => {
  const graph = buildWeightedGraph(
    [node('a', 'doc-a'), node('b', 'doc-a'), node('c', 'doc-c')],
    [
      edge('a', 'b', 0.8),
      { ...edge('b', 'a', 0.9), relation: 'supports' },
      edge('a', 'c', 0.8, ['doc-a', 'doc-c']),
    ],
  );

  assert.equal(graph.edges.length, 2);
  const sameDocument = graph.edges.find(item => item.source === 'a' && item.target === 'b');
  const crossDocument = graph.edges.find(item => item.source === 'a' && item.target === 'c');
  assert.equal(sameDocument.occurrenceCount, 2);
  assert.deepEqual(sameDocument.relations, ['related_to', 'supports']);
  assert.equal(crossDocument.crossDocument, true);
  assert.ok(crossDocument.weight > sameDocument.weight);
  assert.ok(graph.nodes.every(item => item.mass >= 1 && item.importance >= 0 && item.importance <= 1));
});

test('community detection and relationship layout are deterministic and finite', () => {
  const nodes = [
    node('a1', 'doc-a'), node('a2', 'doc-a'), node('a3', 'doc-a'), node('a4', 'doc-a'),
    node('b1', 'doc-b'), node('b2', 'doc-b'), node('b3', 'doc-b'), node('b4', 'doc-b'),
    node('bridge', 'doc-c'),
  ];
  const edges = [
    edge('a1', 'a2'), edge('a1', 'a3'), edge('a1', 'a4'), edge('a2', 'a3'), edge('a2', 'a4'), edge('a3', 'a4'),
    edge('b1', 'b2'), edge('b1', 'b3'), edge('b1', 'b4'), edge('b2', 'b3'), edge('b2', 'b4'), edge('b3', 'b4'),
    edge('a2', 'bridge', 0.75, ['doc-a', 'doc-c']),
    edge('bridge', 'b2', 0.75, ['doc-b', 'doc-c']),
  ];
  const graph = buildWeightedGraph(nodes, edges);
  const firstMemberships = detectWeightedCommunities(graph);
  const secondMemberships = detectWeightedCommunities(graph);
  assert.deepEqual(firstMemberships, secondMemberships);

  const first = calculateGravityLayout(graph, { width: 1200, height: 800, reset: true });
  const second = calculateGravityLayout(graph, { width: 1200, height: 800, reset: true });
  assert.deepEqual(first, second);
  assert.equal(Object.keys(first.positions).length, nodes.length);
  assert.ok(first.clusters.length >= 2);
  assert.ok(first.clusters.some(cluster => cluster.role === 'major'));
  assert.ok(first.clusters.every(cluster => (
    typeof cluster.label === 'string'
    && cluster.label.length > 0
    && cluster.topNodeIds.length > 0
  )));
  assert.equal(Object.keys(first.edgeWeights).length, graph.edges.length);
  assert.ok(Object.values(first.positions).every(position => (
    Number.isFinite(position.x)
    && Number.isFinite(position.y)
    && position.r >= 5
  )));
  assert.ok(first.bounds.maxX > first.bounds.minX);
  assert.ok(first.bounds.maxY > first.bounds.minY);
  for (const cluster of first.clusters) {
    const fitted = fitClusterCircle(cluster.nodeIds, first.positions, {
      padding: 18,
      minimumRadius: 52,
    });
    assert.ok(fitted);
    assert.equal(cluster.x, fitted.x);
    assert.equal(cluster.y, fitted.y);
    assert.equal(cluster.radius, fitted.radius);
  }
});

test('community circle follows asymmetric rendered node bounds', () => {
  const positions = {
    a: { x: 100, y: 50, r: 10 },
    b: { x: 300, y: 90, r: 20 },
    c: { x: 260, y: 200, r: 8 },
  };
  const fitted = fitClusterCircle(['a', 'b', 'c'], positions, {
    padding: 12,
    minimumRadius: 0,
  });

  assert.equal(fitted.x, 205);
  assert.equal(fitted.y, 124);
  for (const position of Object.values(positions)) {
    const outerDistance = Math.hypot(position.x - fitted.x, position.y - fitted.y) + position.r;
    assert.ok(outerDistance + 12 <= fitted.radius + Number.EPSILON * 10);
  }

  const shifted = Object.fromEntries(Object.entries(positions).map(([id, position]) => [id, {
    ...position,
    x: position.x + 75,
    y: position.y - 40,
  }]));
  const shiftedFit = fitClusterCircle(['a', 'b', 'c'], shifted, {
    padding: 12,
    minimumRadius: 0,
  });
  assert.equal(shiftedFit.x, fitted.x + 75);
  assert.equal(shiftedFit.y, fitted.y - 40);
  assert.equal(shiftedFit.radius, fitted.radius);
});

test('large community circle ignores distant outliers and covers most nodes', () => {
  const positions = {};
  const nodeIds = [];
  for (let index = 0; index < 20; index++) {
    const id = `core-${index}`;
    nodeIds.push(id);
    positions[id] = {
      x: (index % 5) * 24,
      y: Math.floor(index / 5) * 24,
      r: 6,
    };
  }
  positions.outlierA = { x: 1_200, y: 900, r: 6 };
  positions.outlierB = { x: -1_000, y: -800, r: 6 };
  nodeIds.push('outlierA', 'outlierB');

  const fitted = fitClusterCircle(nodeIds, positions, {
    padding: 12,
    minimumRadius: 0,
  });
  const enclosed = nodeIds.filter(id => {
    const position = positions[id];
    return Math.hypot(position.x - fitted.x, position.y - fitted.y) + position.r
      <= fitted.radius - 12 + Number.EPSILON * 100;
  });

  assert.ok(enclosed.length >= Math.ceil(nodeIds.length * 0.88));
  assert.ok(fitted.radius < 150);
  assert.ok(fitted.x > 20 && fitted.x < 80);
  assert.ok(fitted.y > 10 && fitted.y < 70);
});

test('small communities use a compact circular node arrangement', () => {
  const nodes = Array.from({ length: 5 }, (_, index) => node(`ring-${index}`, 'ring-doc'));
  const edges = [];
  for (let source = 0; source < nodes.length; source++) {
    for (let target = source + 1; target < nodes.length; target++) {
      edges.push(edge(nodes[source].name, nodes[target].name));
    }
  }
  const graph = buildWeightedGraph(nodes, edges);
  const layout = calculateGravityLayout(graph, { width: 900, height: 700, reset: true });
  const cluster = layout.clusters.find(item => item.nodeIds.length === nodes.length);
  assert.ok(cluster);

  const points = cluster.nodeIds.map(id => layout.positions[id]);
  const centerX = points.reduce((sum, point) => sum + point.x, 0) / points.length;
  const centerY = points.reduce((sum, point) => sum + point.y, 0) / points.length;
  const radii = points.map(point => Math.hypot(point.x - centerX, point.y - centerY));
  assert.ok(Math.min(...radii) > 25);
  assert.ok(Math.max(...radii) - Math.min(...radii) < 1e-8);
  assert.ok(Math.max(...points.map(point => point.x)) - Math.min(...points.map(point => point.x)) > 50);
  assert.ok(Math.max(...points.map(point => point.y)) - Math.min(...points.map(point => point.y)) > 50);
});

test('sparse medium communities form a circular disk instead of a relation chain', () => {
  const nodes = Array.from({ length: 24 }, (_, index) => node(`disk-${index}`, 'disk-doc'));
  const edges = nodes.slice(1).map(item => edge(nodes[0].name, item.name));
  const graph = buildWeightedGraph(nodes, edges);
  const reset = calculateGravityLayout(graph, { width: 900, height: 700, reset: true });
  const cluster = reset.clusters.find(item => item.nodeIds.length === nodes.length);
  assert.ok(cluster);
  assert.ok(cluster.density < 0.45);

  const points = cluster.nodeIds.map(id => reset.positions[id]);
  assert.ok(Math.max(...points.map(point => point.x)) - Math.min(...points.map(point => point.x)) > 120);
  assert.ok(Math.max(...points.map(point => point.y)) - Math.min(...points.map(point => point.y)) > 120);

  const previousPositions = Object.fromEntries(cluster.nodeIds.map((id, index) => [id, {
    x: index * 30,
    y: 0,
  }]));
  const corrected = calculateGravityLayout(graph, {
    width: 900,
    height: 700,
    previousPositions,
    reset: false,
  });
  for (const id of cluster.nodeIds) {
    assert.equal(corrected.positions[id].x, reset.positions[id].x);
    assert.equal(corrected.positions[id].y, reset.positions[id].y);
  }
});

test('established positions receive inertia while reset ignores old coordinates', () => {
  const nodes = Array.from({ length: 13 }, (_, index) => node(`stable-${index}`, 'doc-a'));
  const edges = [];
  for (let source = 0; source < nodes.length; source++) {
    for (let target = source + 1; target < nodes.length; target++) {
      edges.push(edge(nodes[source].name, nodes[target].name));
    }
  }
  const graph = buildWeightedGraph(nodes, edges);
  const reset = calculateGravityLayout(graph, { width: 900, height: 700, reset: true });
  const previousPositions = Object.fromEntries(
    Object.entries(reset.positions).map(([id, position]) => [id, {
      x: position.x + 400,
      y: position.y - 200,
    }]),
  );
  const stabilized = calculateGravityLayout(graph, {
    width: 900,
    height: 700,
    previousPositions,
    reset: false,
  });

  for (const id of Object.keys(reset.positions)) {
    assert.ok(stabilized.positions[id].x > reset.positions[id].x);
    assert.ok(stabilized.positions[id].y < reset.positions[id].y);
  }
});
