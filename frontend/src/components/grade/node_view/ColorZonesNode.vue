<script setup>
import { computed, onBeforeUnmount, ref } from "vue";
import { t } from "../../../i18n/index.js";
import NodeField from "./NodeField.vue";

const props = defineProps({
  node: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["slider", "choice", "toggle", "pick"]);

const TABS = [
  { key: "lightness", label: "grade.colorZonesLightness", max: 100 },
  { key: "chroma", label: "grade.colorZonesChroma", max: 100 },
  { key: "hue", label: "grade.colorZonesHue", max: 180 },
];

const tab = ref("lightness");
const activeTab = computed(() => TABS.find((item) => item.key === tab.value) || TABS[0]);
const svg = ref(null);
let dragIndex = -1;

const W = 360;
const H = 100;
const PAD = 6;

const points = computed(() => {
  const raw = props.node.params?.[activeTab.value.key];
  if (!Array.isArray(raw) || raw.length < 2) {
    return [[0, 0], [360, 0]];
  }
  return raw.map(([hue, value]) => [Number(hue), Number(value)]);
});

function toX(hue) {
  return (hue / 360) * W;
}

function toY(value) {
  const max = activeTab.value.max;
  return H / 2 - (value / max) * (H / 2 - PAD);
}

function fromY(y) {
  const max = activeTab.value.max;
  const value = ((H / 2 - y) / (H / 2 - PAD)) * max;
  return Math.round(Math.max(-max, Math.min(max, value)));
}

// Catmull-Rom through the points, wrapping 360 -> 0, so the drawn curve matches
// what the node evaluates.
function curvePath() {
  const list = points.value;
  const steps = 120;
  const parts = [];
  for (let i = 0; i <= steps; i += 1) {
    const hue = (i / steps) * 360;
    const value = evaluate(list, hue);
    const x = toX(hue);
    const y = toY(value);
    parts.push(`${i === 0 ? "M" : "L"}${x.toFixed(2)},${y.toFixed(2)}`);
  }
  return parts.join(" ");
}

function evaluate(list, hue) {
  const n = list.length;
  let index = n - 1;
  for (let i = 0; i < n; i += 1) {
    const start = list[i][0];
    const end = i + 1 < n ? list[i + 1][0] : list[0][0] + 360;
    if (hue >= start && hue < end) {
      index = i;
      break;
    }
  }
  const [x0, y0] = list[index];
  const [x1raw, y1] = list[(index + 1) % n];
  const x1 = index + 1 < n ? x1raw : x1raw + 360;
  const t = Math.max(0, Math.min(1, (hue - x0) / Math.max(1e-3, x1 - x0)));
  if (props.node.params?.interpolation === "linear") {
    return y0 + (y1 - y0) * t;
  }
  const [xp, yp] = list[(index + n - 1) % n];
  const [xn, yn] = list[(index + 2) % n];
  const m0 = (y1 - yp) / 2;
  const m1 = (yn - y0) / 2;
  const t2 = t * t;
  const t3 = t2 * t;
  return (
    (2 * t3 - 3 * t2 + 1) * y0 + (t3 - 2 * t2 + t) * m0 + (-2 * t3 + 3 * t2) * y1 + (t3 - t2) * m1
  );
}

function commit(next) {
  const sorted = next
    .map(([hue, value]) => [
      Math.max(0, Math.min(360, Math.round(hue))),
      Math.max(-activeTab.value.max, Math.min(activeTab.value.max, Math.round(value))),
    ])
    .sort((a, b) => a[0] - b[0]);
  props.node.params[activeTab.value.key] = sorted;
  emit("pick");
}

function onPointerDown(event, index) {
  event.preventDefault();
  dragIndex = index;
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
}

function onPointerMove(event) {
  if (dragIndex < 0 || !svg.value) return;
  const rect = svg.value.getBoundingClientRect();
  const hue = ((event.clientX - rect.left) / rect.width) * 360;
  const y = ((event.clientY - rect.top) / rect.height) * H;
  const list = points.value.map((entry) => [...entry]);
  const lower = dragIndex > 0 ? list[dragIndex - 1][0] + 1 : 0;
  const upper = dragIndex < list.length - 1 ? list[dragIndex + 1][0] - 1 : 360;
  list[dragIndex][0] = Math.max(lower, Math.min(upper, hue));
  list[dragIndex][1] = Math.max(-activeTab.value.max, Math.min(activeTab.value.max, fromY(y)));
  props.node.params[activeTab.value.key] = list;
}

function onPointerUp() {
  window.removeEventListener("pointermove", onPointerMove);
  if (dragIndex < 0) return;
  dragIndex = -1;
  commit(points.value);
}

function addPoint(event) {
  if (!svg.value) return;
  const rect = svg.value.getBoundingClientRect();
  const hue = ((event.clientX - rect.left) / rect.width) * 360;
  const y = ((event.clientY - rect.top) / rect.height) * H;
  commit([...points.value, [hue, fromY(y)]]);
}

function removePoint(event, index) {
  event.preventDefault();
  if (points.value.length <= 2) return;
  commit(points.value.filter((_, i) => i !== index));
}

function resetCurve() {
  props.node.params[activeTab.value.key] = [[0, 0], [360, 0]];
  emit("pick");
}

onBeforeUnmount(() => {
  window.removeEventListener("pointermove", onPointerMove);
});
</script>

<template>
  <div class="flex flex-col gap-2">
    <div class="flex items-center gap-1" role="tablist">
      <button
        v-for="item in TABS"
        :key="item.key"
        type="button"
        class="flex-1 rounded-md px-2 py-1 text-[11px] transition"
        :class="tab === item.key
          ? (isDark ? 'bg-white/12 text-stone-100' : 'bg-white text-stone-900 shadow-sm')
          : (isDark ? 'text-stone-400 hover:bg-white/6' : 'text-stone-500 hover:bg-white/70')"
        @click="tab = item.key"
      >{{ t(item.label) }}</button>
    </div>

    <div class="relative overflow-hidden rounded-lg border" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
      <div class="hue-strip absolute inset-x-0 top-0 opacity-70" style="height: 42%" />
      <div class="hue-strip absolute inset-x-0 bottom-0 opacity-40" style="height: 42%; filter: brightness(0.55)" />
      <svg
        ref="svg"
        class="relative h-24 w-full cursor-crosshair"
        :viewBox="`0 0 ${W} ${H}`"
        preserveAspectRatio="none"
        @pointerdown.self="addPoint"
      >
        <line :x1="0" :x2="W" :y1="H / 2" :y2="H / 2" :stroke="isDark ? 'rgba(255,255,255,0.35)' : 'rgba(0,0,0,0.25)'" stroke-width="0.6" />
        <path :d="curvePath()" fill="none" :stroke="isDark ? '#fbbf24' : '#b45309'" stroke-width="1.4" />
        <circle
          v-for="([hue, value], index) in points"
          :key="`${hue}:${index}`"
          :cx="toX(hue)"
          :cy="toY(value)"
          r="2.6"
          :fill="isDark ? '#fde68a' : '#92400e'"
          :stroke="isDark ? '#78350f' : '#fff7ed'"
          stroke-width="0.8"
          class="cursor-grab"
          @pointerdown.stop="onPointerDown($event, index)"
          @dblclick.stop="removePoint($event, index)"
        />
      </svg>
    </div>

    <div class="flex items-center justify-between text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">
      <span>{{ t("grade.colorZonesHint") }}</span>
      <button type="button" class="rounded px-1.5 py-0.5 transition" :class="isDark ? 'hover:bg-white/10' : 'hover:bg-stone-200'" @click="resetCurve">
        {{ t("grade.colorZonesReset") }}
      </button>
    </div>

    <NodeField
      v-for="field in node.fields"
      :key="field.key"
      :node="node"
      :field="field"
      :is-dark="isDark"
      @slider="emit('slider', $event)"
      @choice="emit('choice', $event)"
      @toggle="emit('toggle', $event)"
    />
  </div>
</template>

<style scoped>
.hue-strip {
  background: linear-gradient(
    90deg,
    hsl(0 85% 55%) 0%,
    hsl(30 85% 55%) 8%,
    hsl(60 85% 55%) 17%,
    hsl(90 85% 55%) 25%,
    hsl(120 85% 55%) 33%,
    hsl(150 85% 55%) 42%,
    hsl(180 85% 55%) 50%,
    hsl(210 85% 55%) 58%,
    hsl(240 85% 55%) 67%,
    hsl(270 85% 55%) 75%,
    hsl(300 85% 55%) 83%,
    hsl(330 85% 55%) 92%,
    hsl(360 85% 55%) 100%
  );
  pointer-events: none;
}
</style>
