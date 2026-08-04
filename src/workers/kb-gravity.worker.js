import { buildWeightedGraph } from '../lib/kb-graph/graph-model.js';
import { calculateGravityLayout } from '../lib/kb-graph/gravity-layout.js';

self.onmessage = event => {
  const { revision, nodes, edges, options } = event.data || {};
  try {
    const startedAt = performance.now();
    const graph = buildWeightedGraph(nodes, edges);
    const result = calculateGravityLayout(graph, options);
    self.postMessage({
      type: 'layout-result',
      revision,
      result,
      timingMs: performance.now() - startedAt,
    });
  } catch (error) {
    self.postMessage({
      type: 'layout-error',
      revision,
      error: error?.message || String(error),
    });
  }
};

