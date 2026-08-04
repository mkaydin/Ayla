const FALLBACK_PARAMETERS = Object.freeze({
  ParamAngleX: [-30, 30, 0],
  ParamAngleY: [-30, 30, 0],
  ParamAngleZ: [-30, 30, 0],
  ParamBodyAngleX: [-10, 10, 0],
  ParamBodyAngleY: [-10, 10, 0],
  ParamBodyAngleZ: [-10, 10, 0],
  ParamEyeLOpen: [0, 2, 1],
  ParamEyeROpen: [0, 2, 1],
  ParamEyeLSmile: [0, 1, 0],
  ParamEyeRSmile: [0, 1, 0],
  ParamEyeBallX: [-1, 1, 0],
  ParamEyeBallY: [-1, 1, 0],
  ParamBrowLY: [-1, 1, 0],
  ParamBrowRY: [-1, 1, 0],
  ParamBrowLAngle: [-1, 1, 0],
  ParamBrowRAngle: [-1, 1, 0],
  ParamMouthForm: [-1, 1, 0],
  ParamMouthOpenY: [0, 1, 0],
  ParamCheek: [0, 1, 0],
  ParamBreath: [0, 1, 0],
  Param11: [0, 1, 0],
  Param12: [0, 1, 0],
  Param13: [0, 1, 0],
  Param14: [0, 1, 0],
  Param21: [0, 1, 0],
  Param22: [0, 1, 0],
});

function finiteOr(value, fallback) {
  return Number.isFinite(value) ? value : fallback;
}

function readCoreValue(core, method, id, fallback) {
  try {
    return finiteOr(core?.[method]?.(id), fallback);
  } catch {
    return fallback;
  }
}

export class Live2DParameterProfile {
  constructor(core, fallbacks = FALLBACK_PARAMETERS) {
    this.core = core;
    this.parameters = new Map();
    for (const [id, [minimum, maximum, defaultValue]] of Object.entries(fallbacks)) {
      this.parameters.set(id, this._inspect(id, minimum, maximum, defaultValue));
    }
  }

  _inspect(id, fallbackMinimum, fallbackMaximum, fallbackDefault) {
    const minimum = readCoreValue(
      this.core,
      'getParameterMinimumValueById',
      id,
      fallbackMinimum,
    );
    const maximum = readCoreValue(
      this.core,
      'getParameterMaximumValueById',
      id,
      fallbackMaximum,
    );
    const defaultValue = readCoreValue(
      this.core,
      'getParameterDefaultValueById',
      id,
      fallbackDefault,
    );
    return Object.freeze({
      id,
      minimum: Math.min(minimum, maximum),
      maximum: Math.max(minimum, maximum),
      defaultValue,
    });
  }

  get(id) {
    if (!this.parameters.has(id)) {
      const current = readCoreValue(this.core, 'getParameterValueById', id, 0);
      this.parameters.set(id, this._inspect(id, -1, 1, current));
    }
    return this.parameters.get(id);
  }

  defaultValue(id) {
    return this.get(id).defaultValue;
  }

  clamp(id, value) {
    const parameter = this.get(id);
    return Math.min(parameter.maximum, Math.max(parameter.minimum, value));
  }
}

export { FALLBACK_PARAMETERS };
