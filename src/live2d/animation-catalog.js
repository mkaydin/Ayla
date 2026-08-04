const track = (parameter, keyframes, mode = 'add') => Object.freeze({
  parameter,
  mode,
  keyframes: Object.freeze(keyframes.map(([at, value]) => Object.freeze({ at, value }))),
});

const animation = (id, label, durationMs, loop, tracks) => Object.freeze({
  id,
  label,
  durationMs,
  loop,
  tracks: Object.freeze(tracks),
});

export const LIVE2D_EXPRESSIONS = Object.freeze([
  Object.freeze({ id: 'Amazed', label: 'Amazed' }),
  Object.freeze({ id: 'Angry', label: 'Angry' }),
  Object.freeze({ id: 'Cry', label: 'Cry' }),
  Object.freeze({ id: 'Love', label: 'Love' }),
  Object.freeze({ id: 'Nervous', label: 'Nervous' }),
  Object.freeze({ id: 'Sleepy', label: 'Sleepy' }),
  Object.freeze({ id: 'Customise', label: 'Customise' }),
]);

export const LIVE2D_ANIMATIONS = Object.freeze([
  animation('neutral-idle', 'Neutral Idle', 5200, true, [
    track('ParamAngleX', [[0, 0], [0.25, 2.2], [0.5, 0], [0.75, -2.2], [1, 0]]),
    track('ParamAngleZ', [[0, 0], [0.5, 0.8], [1, 0]]),
    track('ParamBodyAngleX', [[0, 0], [0.25, 0.8], [0.75, -0.8], [1, 0]]),
  ]),
  animation('attentive-idle', 'Attentive Idle', 4200, true, [
    track('ParamAngleY', [[0, 0], [0.4, 1.8], [1, 0]]),
    track('ParamEyeBallX', [[0, 0], [0.35, 0.18], [0.7, -0.12], [1, 0]]),
    track('ParamEyeBallY', [[0, 0.05], [0.5, 0.14], [1, 0.05]]),
  ]),
  animation('thinking', 'Thinking', 3600, true, [
    track('ParamAngleX', [[0, 0], [0.3, 5], [0.8, 5], [1, 0]]),
    track('ParamAngleY', [[0, 0], [0.3, 3], [0.8, 3], [1, 0]]),
    track('ParamEyeBallX', [[0, 0], [0.3, 0.45], [0.8, 0.45], [1, 0]]),
    track('ParamEyeBallY', [[0, 0], [0.3, 0.3], [0.8, 0.3], [1, 0]]),
  ]),
  animation('nod', 'Nod', 900, false, [
    track('ParamAngleY', [[0, 0], [0.3, -8], [0.55, 5], [0.8, -3], [1, 0]]),
    track('ParamBodyAngleY', [[0, 0], [0.35, -2], [0.65, 1], [1, 0]]),
  ]),
  animation('head-shake', 'Head Shake', 1050, false, [
    track('ParamAngleX', [[0, 0], [0.2, -10], [0.45, 10], [0.7, -8], [0.9, 5], [1, 0]]),
    track('ParamAngleZ', [[0, 0], [0.2, 2], [0.45, -2], [0.7, 1.5], [1, 0]]),
  ]),
  animation('look-around', 'Look Around', 3600, false, [
    track('ParamAngleX', [[0, 0], [0.25, -12], [0.5, -12], [0.75, 12], [0.92, 12], [1, 0]]),
    track('ParamEyeBallX', [[0, 0], [0.2, -0.65], [0.5, -0.65], [0.7, 0.65], [0.92, 0.65], [1, 0]]),
  ]),
  animation('happy-bounce', 'Happy Bounce', 1200, false, [
    track('ParamBodyAngleY', [[0, 0], [0.2, 3], [0.4, -1], [0.6, 3], [0.8, -1], [1, 0]]),
    track('ParamBodyAngleZ', [[0, 0], [0.25, -2], [0.55, 2], [0.8, -1], [1, 0]]),
    track('ParamEyeLSmile', [[0, 0], [0.2, 0.8], [0.8, 0.8], [1, 0]]),
    track('ParamEyeRSmile', [[0, 0], [0.2, 0.8], [0.8, 0.8], [1, 0]]),
    track('ParamMouthForm', [[0, 0], [0.2, 0.65], [0.8, 0.65], [1, 0]]),
  ]),
  animation('surprise-recoil', 'Surprise Recoil', 950, false, [
    track('ParamAngleY', [[0, 0], [0.18, 10], [0.55, 7], [1, 0]]),
    track('ParamBodyAngleY', [[0, 0], [0.18, -3], [0.6, -2], [1, 0]]),
    track('Param21', [[0, 0], [0.12, 1], [0.7, 1], [1, 0]]),
  ]),
  animation('sleepy-sway', 'Sleepy Sway', 5000, true, [
    track('ParamAngleZ', [[0, 0], [0.3, -5], [0.7, 4], [1, 0]]),
    track('ParamBodyAngleX', [[0, 0], [0.3, -2], [0.7, 2], [1, 0]]),
    track('Param12', [[0, 0.75], [0.8, 0.75], [1, 0.75]]),
  ]),
  animation('nervous-sway', 'Nervous Sway', 2600, true, [
    track('ParamAngleX', [[0, 0], [0.2, -3], [0.4, 2], [0.6, -2], [0.8, 3], [1, 0]]),
    track('Param14', [[0, 0.8], [0.8, 0.8], [1, 0.8]]),
  ]),
]);

export const LIVE2D_ANIMATION_CATALOG = new Map(
  LIVE2D_ANIMATIONS.map(definition => [definition.id, definition]),
);

export function getLive2DAnimation(id) {
  return LIVE2D_ANIMATION_CATALOG.get(id) || null;
}
