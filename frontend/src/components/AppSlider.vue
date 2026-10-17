<script setup>
import { computed, onBeforeUnmount, ref } from "vue";

const props = defineProps({
  modelValue: { type: Number, required: true },
  min: { type: Number, default: 0 },
  max: { type: Number, default: 100 },
  step: { type: [Number, String], default: 1 },
  tick: { type: Number, default: null },
  tickTitle: { type: String, default: "" },
  isDark: { type: Boolean, default: false },
  tone: { type: String, default: "amber" },
  ariaLabel: { type: String, default: "" },
  disabled: { type: Boolean, default: false },
});

const emit = defineEmits(["update:modelValue", "input", "change"]);

const dragging = ref(false);
const span = computed(() => {
  const range = props.max - props.min;
  return range === 0 ? 1 : range;
});
const percent = computed(() => {
  const value = Number(props.modelValue);
  const n = Number.isFinite(value) ? value : props.min;
  return ((n - props.min) / span.value) * 100;
});
const tickPercent = computed(() => {
  if (props.tick == null || !Number.isFinite(props.tick)) return null;
  return ((props.tick - props.min) / span.value) * 100;
});

function parseValue(raw) {
  return Number(raw);
}

function onInput(event) {
  const value = parseValue(event.target.value);
  emit("update:modelValue", value);
  emit("input", value);
}

function onChange(event) {
  emit("change", parseValue(event.target.value));
}

function onPointerDown() {
  dragging.value = true;
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerUp);
}

function onPointerUp() {
  dragging.value = false;
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);
}

onBeforeUnmount(onPointerUp);
</script>

<template>
  <div
    class="app-slider"
    :class="[
      dragging ? 'is-dragging' : '',
      tone === 'ghost' ? 'is-ghost' : '',
      isDark ? 'is-dark' : '',
    ]"
  >
    <div class="app-slider-track">
      <div class="app-slider-fill" :style="{ width: `${percent}%` }" />
      <div
        v-if="tickPercent != null"
        class="app-slider-tick"
        :style="{ left: `${tickPercent}%` }"
        :title="tickTitle || undefined"
      />
    </div>
    <div class="app-slider-thumb" :style="{ left: `${percent}%` }" />
    <input
      type="range"
      class="app-slider-input"
      :min="min"
      :max="max"
      :step="step"
      :value="modelValue"
      :aria-label="ariaLabel || $attrs['aria-label'] || undefined"
      :disabled="disabled"
      @input="onInput"
      @change="onChange"
      @pointerdown="onPointerDown"
    />
  </div>
</template>

<style scoped>
.app-slider {
  --slider-track: rgb(214 211 209);
  --slider-fill: var(--color-amber-500);
  --slider-thumb: var(--color-amber-500);
  --slider-tick: rgb(120 113 108);
  position: relative;
  display: flex;
  height: 1.25rem;
  max-height: 1.25rem;
  min-height: 1.25rem;
  width: 100%;
  min-width: 0;
  flex: 1 1 auto;
  align-items: center;
}

.app-slider.is-dark {
  --slider-track: rgb(68 64 60);
  --slider-tick: rgb(168 162 158);
}

.app-slider.is-ghost {
  --slider-track: color-mix(in srgb, currentColor 22%, transparent);
  --slider-fill: color-mix(in srgb, currentColor 55%, transparent);
  --slider-thumb: currentColor;
  --slider-tick: color-mix(in srgb, currentColor 55%, transparent);
}

.app-slider-track {
  position: relative;
  height: 0.25rem;
  width: 100%;
  overflow: visible;
  border-radius: 999px;
  background: var(--slider-track);
}

.app-slider-fill {
  position: absolute;
  inset: 0 auto 0 0;
  border-radius: inherit;
  background: var(--slider-fill);
}

.app-slider-tick {
  position: absolute;
  top: 50%;
  z-index: 1;
  width: 2px;
  height: 0.7rem;
  transform: translate(-50%, -50%);
  border-radius: 999px;
  background: var(--slider-tick);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--slider-track) 70%, transparent);
}

.app-slider-thumb {
  position: absolute;
  top: 50%;
  z-index: 2;
  width: 0.85rem;
  height: 0.85rem;
  pointer-events: none;
  border-radius: 999px;
  background: var(--slider-thumb);
  box-shadow: 0 1px 2px rgb(0 0 0 / 0.22);
  transform: translate(-50%, -50%);
}

.app-slider-input {
  position: absolute;
  inset: 0;
  z-index: 3;
  margin: 0;
  cursor: pointer;
  opacity: 0;
  appearance: none;
  -webkit-appearance: none;
  background: transparent;
}

.app-slider-input::-webkit-slider-runnable-track {
  appearance: none;
  -webkit-appearance: none;
  height: 100%;
  border: none;
  background: transparent;
}

.app-slider-input::-webkit-slider-thumb {
  appearance: none;
  -webkit-appearance: none;
  width: 1.25rem;
  height: 1.25rem;
  border: none;
  background: transparent;
  box-shadow: none;
}

.app-slider-input::-moz-range-track,
.app-slider-input::-moz-range-progress {
  height: 100%;
  border: none;
  background: transparent;
}

.app-slider-input::-moz-range-thumb {
  width: 1.25rem;
  height: 1.25rem;
  border: none;
  background: transparent;
  box-shadow: none;
}

.app-slider-fill,
.app-slider-thumb {
  transition:
    width 220ms cubic-bezier(0.22, 1, 0.36, 1),
    left 220ms cubic-bezier(0.22, 1, 0.36, 1);
}

.app-slider.is-dragging .app-slider-fill,
.app-slider.is-dragging .app-slider-thumb {
  transition: none;
}
</style>
