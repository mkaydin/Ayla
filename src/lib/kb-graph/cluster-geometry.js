/**
 * Fit a visual community circle to the node positions that are actually being
 * rendered. Using the layout simulation's original center becomes inaccurate
 * after position inertia or manual dragging.
 */
export function fitClusterCircle(
  nodeIds = [],
  positions = {},
  {
    padding = 18,
    minimumRadius = 52,
    coverage = 0.88,
    robustFitThreshold = 10,
  } = {},
) {
  const nodes = [];
  let left = Infinity;
  let right = -Infinity;
  let top = Infinity;
  let bottom = -Infinity;

  for (const id of nodeIds) {
    const position = positions[id];
    if (!Number.isFinite(position?.x) || !Number.isFinite(position?.y)) continue;
    const radius = Math.max(0, Number(position.r) || 0);
    nodes.push({ x: position.x, y: position.y, radius });
    left = Math.min(left, position.x - radius);
    right = Math.max(right, position.x + radius);
    top = Math.min(top, position.y - radius);
    bottom = Math.max(bottom, position.y + radius);
  }
  if (!nodes.length) return null;

  // Small communities should remain fully enclosed. Large communities use a
  // robust center and percentile radius so one stale/dragged node cannot turn
  // the background into an enormous circle.
  let x = (left + right) / 2;
  let y = (top + bottom) / 2;
  const useRobustFit = nodes.length >= robustFitThreshold;
  const coveredCount = useRobustFit
    ? Math.max(1, Math.min(nodes.length, Math.ceil(nodes.length * coverage)))
    : nodes.length;

  if (useRobustFit) {
    const median = values => {
      const sorted = [...values].sort((a, b) => a - b);
      const middle = Math.floor(sorted.length / 2);
      return sorted.length % 2
        ? sorted[middle]
        : (sorted[middle - 1] + sorted[middle]) / 2;
    };
    const medianX = median(nodes.map(node => node.x));
    const medianY = median(nodes.map(node => node.y));
    const nearest = [...nodes]
      .sort((a, b) => (
        Math.hypot(a.x - medianX, a.y - medianY)
        - Math.hypot(b.x - medianX, b.y - medianY)
      ))
      .slice(0, coveredCount);
    x = nearest.reduce((sum, node) => sum + node.x, 0) / nearest.length;
    y = nearest.reduce((sum, node) => sum + node.y, 0) / nearest.length;
  }

  const outerDistances = nodes
    .map(node => Math.hypot(node.x - x, node.y - y) + node.radius)
    .sort((a, b) => a - b);
  const requiredRadius = outerDistances[coveredCount - 1];

  return {
    x,
    y,
    radius: Math.max(minimumRadius, requiredRadius + Math.max(0, padding)),
  };
}
