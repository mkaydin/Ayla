import { Container, Point, Rectangle } from 'pixi.js';
import { MK6_EXPRESSIONS, MK6_EXPRESSION_DATA } from './mk6-profile.js';

// An empty scene keeps speech and panels independent of optional licensed assets.
// It has no rendered character and never fetches executable code from a CDN.
export function createAvatarFreeScene() {
  const scene = new Container();
  scene.anchor = new Point();
  scene.getLocalBounds = () => new Rectangle(0, 0, 1, 1);
  scene.internalModel = new Container();
  scene.internalModel.coreModel = {
    setParameterValueById() {},
    addParameterValueById() {},
    multiplyParameterValueById() {},
    getParameterValueById() { return 0; },
  };
  scene.internalModel.motionManager = { stopAllMotions() {} };
  scene.motion = async () => false;
  scene[MK6_EXPRESSION_DATA] = new Map(MK6_EXPRESSIONS.map(({ id }) => [id, []]));
  scene.avatarAvailable = false;
  return scene;
}
