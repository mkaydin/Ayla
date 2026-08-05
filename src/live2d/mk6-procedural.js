import { MK6_PARAMETER_IDS } from './mk6-profile.js';

const clamp = (value, minimum, maximum) => Math.max(minimum, Math.min(maximum, value));
const smooth = (current, target, deltaMs, responseMs) => {
  const alpha = 1 - Math.exp(-Math.max(0, deltaMs) / responseMs);
  return current + (target - current) * alpha;
};

// Sleep and tracking-lost author facial/effect parameters that the normal Idle
// motion never touches. Cubism consequently saves their final values. Reset
// only those neutral-base parameters before the next normal motion update;
// active layered expressions are applied later in beforeModelUpdate.
const AUTHORED_POSE_RESIDUE = Object.freeze([
  'ParamBrowLDown',
  'ParamBrowRDown',
  'ParamBrowLOutterUp',
  'BrowROutterUp',
  'BrowRInnerUp',
  'ParamBrowForm',
  'ParamMouthForm',
  'ParamMouthFunnel',
  'ParamMouthPressLipOpen',
  'ParamMouthPunkerWiden',
  'ParamMouthShrug',
  'ParamMouthX',
  'ParamCheekPuff',
  'ParamMode2Form',
  'ParamMouthMode',
  'ExpHairSticker',
  'ExpMask',
  'ExpSleeping',
  'ExpMouthSolidcolor',
]);

/**
 * Adds the small movements that are not authored in Mk6's three motion files.
 * Values are intentionally conservative: Cubism physics turns these drivers
 * into secondary hair, clothing, hand, and leg movement.
 */
export class Mk6ProceduralAnimator {
  constructor(coreModel, parameterIds = MK6_PARAMETER_IDS) {
    this.core = coreModel;
    this.ids = parameterIds;
    this.pointerX = 0;
    this.pointerY = 0;
    this.headX = 0;
    this.headY = 0;
    this.bodyX = 0;
    this.bodyY = 0;
    this.bodyZ = 0;
    this.workX = 0;
    this.workY = 0;
  }

  setPointerNormalized(x, y) {
    this.pointerX = clamp(Number(x) || 0, -1, 1);
    this.pointerY = clamp(Number(y) || 0, -1, 1);
  }

  resetAuthoredPoseResidue() {
    for (const id of AUTHORED_POSE_RESIDUE) this._set(id, 0);
  }

  update(deltaMs, nowMs, { enabled = true, working = false } = {}) {
    if (!enabled || !this.core) return;
    const now = Math.max(0, Number(nowMs) || 0);
    const cursorStrength = working ? 0.42 : 1;
    const idleSway = Math.sin(now / 3_900);
    const slowSway = Math.sin(now / 7_300 + 0.8);

    this.headX = smooth(this.headX, this.pointerX * 30 * cursorStrength, deltaMs, 115);
    this.headY = smooth(this.headY, -this.pointerY * 24 * cursorStrength, deltaMs, 135);
    this.bodyX = smooth(
      this.bodyX,
      this.pointerX * 8 * cursorStrength + idleSway * 1.15,
      deltaMs,
      430,
    );
    this.bodyY = smooth(
      this.bodyY,
      -this.pointerY * 3.5 * cursorStrength + slowSway * 0.55,
      deltaMs,
      520,
    );
    this.bodyZ = smooth(
      this.bodyZ,
      -this.pointerX * 4.5 * cursorStrength + idleSway * 0.8,
      deltaMs,
      560,
    );

    this._set(this.ids.headX, this.headX);
    this._set(this.ids.headY, this.headY);
    this._set(this.ids.headZ, -this.headX * 0.18 + idleSway * 0.35);
    this._set(this.ids.bodyX, this.bodyX + (working ? -1.1 : 0));
    this._set(this.ids.bodyY, this.bodyY + (working ? 0.7 : 0));
    this._set(this.ids.bodyZ, this.bodyZ + (working ? -1.2 : 0));
    this._set(this.ids.eyeX, this.pointerX * cursorStrength);
    this._set(this.ids.eyeY, -this.pointerY * cursorStrength);
    this._set(this.ids.breath, Math.sin(now / 1_450) * 0.5 + 0.5);

    // Weight shifts keep the lower body alive. Offset phases avoid the four
    // leg inputs collapsing into the same mechanical line of motion.
    const weight = Math.sin(now / 4_800);
    const step = Math.sin(now / 6_300 + 1.2);
    this._set(this.ids.rightLegX, weight * 0.18);
    this._set(this.ids.leftLegX, -weight * 0.16);
    this._set(this.ids.rightLegY, step * 0.12 + (working ? -0.04 : 0));
    this._set(this.ids.leftLegY, -step * 0.1 + (working ? 0.03 : 0));

    if (working) this._updateWriting(deltaMs, now);
    else this._settleWriting(deltaMs);
  }

  _updateWriting(deltaMs, now) {
    // A six-second work phrase: several strokes, a pause to read, then a
    // reposition. The two incommensurate waves keep repetitions unobtrusive.
    const phrase = (now % 6_200) / 6_200;
    const writing = phrase < 0.68;
    const strokeX = writing
      ? Math.sin(now / 155) * 0.34 + Math.sin(now / 410) * 0.1
      : phrase < 0.84 ? 0.18 : -0.12;
    const row = writing
      ? Math.sin(now / 690) * 0.16 - Math.floor((phrase * 5) % 3) * 0.055
      : 0.08;
    this.workX = smooth(this.workX, strokeX, deltaMs, writing ? 75 : 260);
    this.workY = smooth(this.workY, row, deltaMs, writing ? 105 : 300);
    this._set(this.ids.mousePositionX, this.workX);
    this._set(this.ids.mousePositionY, this.workY);
    this._set(this.ids.leftArmWork, this.workX * 0.32 + this.workY * 0.12);
  }

  _settleWriting(deltaMs) {
    this.workX = smooth(this.workX, 0, deltaMs, 240);
    this.workY = smooth(this.workY, 0, deltaMs, 240);
    this._set(this.ids.mousePositionX, this.workX);
    this._set(this.ids.mousePositionY, this.workY);
    this._set(this.ids.leftArmWork, this.workX * 0.32);
  }

  _set(id, value, weight = 1) {
    if (id) this.core.setParameterValueById(id, value, weight);
  }
}
