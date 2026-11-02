import InputNode from "./InputNode.vue";
import WhiteBalanceNode from "./WhiteBalanceNode.vue";
import DemosaicNode from "./DemosaicNode.vue";
import InputColorProfileNode from "./InputColorProfileNode.vue";
import ExposureNode from "./ExposureNode.vue";
import LocalContrastNode from "./LocalContrastNode.vue";
import DehazeNode from "./DehazeNode.vue";
import VelviaNode from "./VelviaNode.vue";
import DenoiseNode from "./DenoiseNode.vue";
import SharpenNode from "./SharpenNode.vue";
import RotateNode from "./RotateNode.vue";
import CropNode from "./CropNode.vue";
import SigmoidNode from "./SigmoidNode.vue";
import LutNode from "./LutNode.vue";
import ColorZonesNode from "./ColorZonesNode.vue";
import ExportNode from "./ExportNode.vue";
import GenericNode from "./GenericNode.vue";

const views = {
  "input": InputNode,
  "white-balance": WhiteBalanceNode,
  "demosaic": DemosaicNode,
  "input-color-profile": InputColorProfileNode,
  "exposure": ExposureNode,
  "local-contrast": LocalContrastNode,
  "dehaze": DehazeNode,
  "velvia": VelviaNode,
  "denoise": DenoiseNode,
  "sharpen": SharpenNode,
  "rotate": RotateNode,
  "crop": CropNode,
  "sigmoid": SigmoidNode,
  "lut": LutNode,
  "color-zones": ColorZonesNode,
  "export": ExportNode,
};

export function nodeView(id) {
  return views[id] || GenericNode;
}
