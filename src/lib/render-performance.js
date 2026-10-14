const RESOLUTION_STEPS = [0.5, 0.625, 0.75, 1, 1.25, 1.5, 2];

// Pixi 6 truncates elapsed time to integer milliseconds before applying its
// FPS limit. Use an integer period so a nominal 60 FPS limit does not wait
// for 17 ms and regularly miss the third refresh on a 180 Hz display.
export function tickerFpsLimit(target) {
  return 1000 / Math.max(1, Math.floor(1000 / target));
}

export const RENDER_QUALITY = {
  auto: { label: 'Auto', pixelBudget: 4_000_000, maxResolution: 1.5 },
  quality: { label: 'Quality', pixelBudget: 8_000_000, maxResolution: 2 },
  performance: { label: 'Performance', pixelBudget: 2_000_000, maxResolution: 1 },
};

export function normalizeRenderQuality(value) {
  return Object.hasOwn(RENDER_QUALITY, value) ? value : 'auto';
}

export function selectRenderResolution({
  width,
  height,
  devicePixelRatio = 1,
  quality = 'auto',
  resizing = false,
}) {
  const preset = RENDER_QUALITY[normalizeRenderQuality(quality)];
  const cssPixels = Math.max(1, width) * Math.max(1, height);
  const budgetResolution = Math.sqrt(preset.pixelBudget / cssPixels);
  let target = Math.min(
    Math.max(0.5, devicePixelRatio || 1),
    preset.maxResolution,
    budgetResolution,
  );
  if (resizing) target *= 0.72;

  let selected = RESOLUTION_STEPS[0];
  for (const step of RESOLUTION_STEPS) {
    if (step > target) break;
    selected = step;
  }
  return selected;
}

export function framebufferPixels(width, height, resolution) {
  return Math.round(Math.max(1, width) * Math.max(1, height) * resolution * resolution);
}
