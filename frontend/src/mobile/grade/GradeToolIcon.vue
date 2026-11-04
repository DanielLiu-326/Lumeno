<script setup>
import {
  IconContrast, IconDroplet, IconHaze, IconSliders, IconSparkles,
  IconSun, IconThermometer, IconTriangle, IconWand,
} from "../../utils/icons";

const props = defineProps({
  kind: { type: String, required: true },
  size: { type: Number, default: 22 },
});

const ICONS = {
  tone: IconSun,
  color: IconSparkles,
  correct: IconWand,
  base: IconThermometer,
  exposure: IconSun,
  "local-contrast": IconContrast,
  sigmoid: IconSliders,
  velvia: IconSparkles,
  dehaze: IconHaze,
  denoise: IconDroplet,
  sharpen: IconTriangle,
  "white-balance": IconThermometer,
};

const icon = ICONS[props.kind] || IconSliders;
</script>

<template>
  <component :is="icon" :style="{ width: `${size}px`, height: `${size}px` }" />
</template>
