
export const MK6_MODEL_URL = '/__ayla_model__/Mk6.model3.json';
export const MK6_EXPRESSION_DATA = Symbol('ayla.mk6.expressionData');

const expression = (id, label, channel, slots = [channel]) => Object.freeze({
  id,
  label,
  channel,
  slots: Object.freeze(slots),
});

export const MK6_EXPRESSIONS = Object.freeze([
  expression('Angry', 'Angry', 'emotion'),
  expression('Confused', 'Confused', 'emotion'),
  expression('Embarrassed', 'Embarrassed', 'emotion'),
  expression('ExtremelyTired', 'Extremely Tired', 'emotion'),
  expression('Happy', 'Happy', 'emotion'),
  expression('Messy', 'Messy', 'emotion'),
  expression('Murderous', 'Murderous', 'emotion'),
  expression('Surprised', 'Surprised', 'emotion'),
  expression('Tired', 'Tired', 'emotion'),
  expression('Blush', 'Blush', 'modifier', ['blush']),
  expression('Exclamation', 'Exclamation', 'indicator'),
  expression('Question', 'Question', 'indicator'),
  expression('Loading', 'Loading', 'indicator'),
  expression('Eyemask', 'Eye Mask', 'appearance', ['face-overlay']),
  expression('JaketOFF', 'Jacket Off', 'appearance', ['outfit']),
  expression('Outfit', 'Alternate Outfit', 'appearance', ['outfit']),
  expression('Controller', 'Controller', 'prop', ['right-hand', 'left-hand']),
  expression('Pen', 'Pen', 'prop', ['left-hand']),
  expression('Tablet', 'Tablet', 'prop', ['right-hand']),
  expression('Tracking', 'Tracking Hands', 'prop', ['right-hand', 'left-hand']),
  expression('MouseClick', 'Mouse Click', 'interaction'),
]);

export const MK6_MOTIONS = Object.freeze([
  Object.freeze({ id: 'idle', label: 'Idle Effects', group: 'Idle', index: 0, loop: true }),
  Object.freeze({ id: 'tracking-lost', label: 'Tracking Lost', group: 'TrackingLost', index: 0, loop: true }),
  Object.freeze({ id: 'sleep', label: 'Sleep', group: 'Sleep', index: 0, loop: true }),
]);

export const MK6_PARAMETER_IDS = Object.freeze({
  headX: 'ParamAngleX',
  headY: 'ParamAngleY',
  headZ: 'ParamAngleZ',
  bodyX: 'ParamBodyAngleX',
  bodyY: 'BodyAngleY',
  bodyZ: 'ParamBodyAngleZ',
  eyeX: 'ParamEyeBallX',
  eyeY: 'ParamEyeBallY',
  eyeLeftOpen: 'ParamEyeLOpen',
  eyeRightOpen: 'ParamEyeROpen',
  mouthOpen: 'ParamMouthOpenY',
  jawOpen: 'ParamJawOpen',
  breath: 'ParamBreath',
  mousePositionX: 'ParamMousePostionX',
  mousePositionY: 'ParamMousePostionY',
  leftArmWork: 'ParamArmL2',
  rightLegX: 'ParamLegX',
  leftLegX: 'ParamLegLX',
  rightLegY: 'ParamLegRY',
  leftLegY: 'ParamLegLY',
});

export function augmentMk6Manifest(source, manifestUrl = MK6_MODEL_URL) {
  if (!source?.FileReferences?.Moc || !Array.isArray(source.FileReferences.Textures)) {
    throw new TypeError('Mk6 model manifest is invalid');
  }
  const manifest = structuredClone(source);
  manifest.url = manifestUrl;
  manifest.FileReferences.Expressions = MK6_EXPRESSIONS.map(item => ({
    Name: item.id,
    File: `Expressions/${item.id}.exp3.json`,
  }));
  manifest.FileReferences.Motions = {
    Idle: [{ File: 'Motions/idle.motion3.json', FadeInTime: 0.35, FadeOutTime: 0.35 }],
    TrackingLost: [{ File: 'Motions/idle_trackinglost.motion3.json', FadeInTime: 0.4, FadeOutTime: 0.4 }],
    Sleep: [{ File: 'Motions/Sleep.motion3.json', FadeInTime: 0.8, FadeOutTime: 0.8 }],
  };
  // Mk6 ships empty groups. Ayla owns blink and TTS explicitly, so retaining
  // empty groups prevents Cubism's automatic layers from fighting the mixer.
  manifest.Groups = [
    { Target: 'Parameter', Name: 'EyeBlink', Ids: [] },
    { Target: 'Parameter', Name: 'LipSync', Ids: [] },
  ];
  return manifest;
}

function parseExpressionData(id, source) {
  if (!Array.isArray(source?.Parameters)) {
    throw new TypeError(`Mk6 expression '${id}' has no parameter array`);
  }
  const parameters = source.Parameters.map(parameter => Object.freeze({
    id: String(parameter.Id),
    value: Number(parameter.Value),
    blend: String(parameter.Blend || 'Add').toLowerCase(),
  }));

  // Mk6's prop expressions only enable their alternate arm artwork. The
  // default R1/L1 arm pair remains enabled unless the host explicitly hides
  // it, which produces three or four visible arms. Overwrite is temporary in
  // Cubism's expression pass, so the normal arms fade back when the prop lease
  // ends without changing the model's saved base parameters.
  const hiddenDefaultArms = {
    Tablet: ['ParamForearmR1ONOFF'],
    Pen: ['ParamForearmL1ONOFF'],
    Controller: ['ParamForearmR1ONOFF', 'ParamForearmL1ONOFF'],
    Tracking: ['ParamForearmR1ONOFF', 'ParamForearmL1ONOFF'],
  }[id] || [];
  for (const parameterId of hiddenDefaultArms) {
    parameters.push(Object.freeze({
      id: parameterId,
      // Mk6 uses 0 for its visible base arm and 1 for the hidden state.
      value: 1,
      blend: 'overwrite',
    }));
  }
  return Object.freeze(parameters);
}

function attachExpressionData(model, expressionData) {
  Object.defineProperty(model, MK6_EXPRESSION_DATA, {
    configurable: false,
    enumerable: false,
    writable: false,
    value: expressionData,
  });
  return model;
}

async function expressionDataFromFetch(fetchImpl) {
  const entries = await Promise.all(MK6_EXPRESSIONS.map(async definition => {
    const path = `${MK6_MODEL_URL.replace(/\/Mk6\.model3\.json$/, '')}/Expressions/${definition.id}.exp3.json`;
    const response = await fetchImpl(path, { cache: 'no-store' });
    if (!response.ok) throw new Error(`${definition.id} expression: HTTP ${response.status}`);
    return [definition.id, parseExpressionData(definition.id, await response.json())];
  }));
  return new Map(entries);
}

function disableNativeExpressions(manifest) {
  // The Cubism expression manager has one global slot. Ayla mixes the same
  // authored parameters by independent semantic slots instead.
  manifest.FileReferences.Expressions = [];
  return manifest;
}

export async function loadMk6Model(Live2DModel, fetchImpl = fetch) {
  try {
    const response = await fetchImpl(MK6_MODEL_URL, { cache: 'no-store' });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const manifest = augmentMk6Manifest(await response.json(), MK6_MODEL_URL);
    const expressionData = await expressionDataFromFetch(fetchImpl);
    disableNativeExpressions(manifest);
    const model = await Live2DModel.from(manifest, {
      autoInteract: false,
      autoUpdate: true,
      idleMotionGroup: 'Idle',
      motionPreload: 'ALL',
    });
    return attachExpressionData(model, expressionData);
  } catch (networkError) {
    throw new Error(
      `Mk6 model is unavailable (${networkError.message}). Install Mk6_1.0 locally or set AYLA_LIVE2D_MODEL_DIR.`,
    );
  }
}
