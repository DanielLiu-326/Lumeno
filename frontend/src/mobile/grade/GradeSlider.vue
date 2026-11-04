<script setup>
import { computed, onBeforeUnmount, ref } from "vue";

const props = defineProps({
  label: { type: String, required: true },
  modelValue: { type: Number, required: true },
  min: { type: Number, default: 0 },
  max: { type: Number, default: 100 },
  step: { type: Number, default: 0 },
  display: { type: String, default: "" },
  gradient: { type: String, default: "" },
  disabled: { type: Boolean, default: false },
});

const emit = defineEmits(["start", "input", "end", "reset"]);

const track = ref(null);
let dragging = false;
let pointerId = null;
let longPress = 0;
let lastTap = 0;

const span = computed(() => (props.max - props.min) || 1);
const percent = computed(() => {
  const value = Number(props.modelValue);
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, ((value - props.min) / span.value) * 100));
});

function valueAt(event) {
  const rect = track.value.getBoundingClientRect();
  const ratio = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
  const raw = props.min + ratio * span.value;
  const step = props.step > 0 ? props.step : span.value / 400;
  const snapped = Math.round(raw / step) * step;
  return Math.max(props.min, Math.min(props.max, Number(snapped.toFixed(6))));
}

function onPointerDown(event) {
  if (props.disabled) return;
  event.preventDefault();
  track.value.setPointerCapture?.(event.pointerId);
  dragging = true;
  pointerId = event.pointerId;
  const now = Date.now();
  if (now - lastTap < 300) reset();
  lastTap = now;
  clearTimeout(longPress);
  longPress = window.setTimeout(() => {
    if (!dragging) return;
    reset();
  }, 600);
  emit("start");
  emit("input", valueAt(event));
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
  window.addEventListener("pointercancel", onPointerUp, { once: true });
}

function onPointerMove(event) {
  if (!dragging || (pointerId !== null && event.pointerId !== pointerId)) return;
  emit("input", valueAt(event));
}

function onPointerUp() {
  clearTimeout(longPress);
  window.removeEventListener("pointermove", onPointerMove);
  if (!dragging) return;
  dragging = false;
  emit("end");
}

function reset() {
  clearTimeout(longPress);
  window.removeEventListener("pointermove", onPointerMove);
  dragging = false;
  emit("reset");
  emit("end");
}

onBeforeUnmount(() => {
  clearTimeout(longPress);
  window.removeEventListener("pointermove", onPointerMove);
});
</script>

<template>
  <div class="flex flex-col gap-1.5" :class="disabled ? 'opacity-40' : ''">
    <div class="flex items-baseline justify-between">
      <span class="text-[0.8125rem] text-stone-300">{{ label }}</span>
      <span class="tabular-nums text-[0.75rem] text-stone-400">{{ display }}</span>
    </div>
    <div
      ref="track"
      class="relative h-6 select-none touch-none"
      :class="disabled ? '' : 'cursor-pointer'"
      @pointerdown="onPointerDown"
    >
      <div class="absolute inset-x-0 top-1/2 h-1.5 -translate-y-1/2 overflow-hidden rounded-full bg-white/12">
        <div v-if="gradient" class="h-full w-full" :style="{ background: gradient }" />
      </div>
      <div
        class="absolute top-1/2 h-5 w-5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white/80 bg-[#1b1b1b] shadow-[0_1px_6px_rgba(0,0,0,0.5)]"
        :style="{ left: `${percent}%` }"
      />
    </div>
  </div>
</template>
