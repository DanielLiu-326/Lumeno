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
const HIT_PX = 16;

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

// Background per tab: the y axis is what that curve changes, at the hue of that
// column. Lightness runs black → hue → white, chroma desaturated → saturated,
// hue is the colour wheel shifted vertically.
const GRID_X = 24;
const GRID_Y = 8;
const background = computed(() => {
  const cells = [];
  for (let row = 0; row < GRID_Y; row += 1) {
    const down = (row + 0.5) / GRID_Y; // 0 = top, 1 = bottom
    for (let col = 0; col < GRID_X; col += 1) {
      const hue = ((col + 0.5) / GRID_X) * 360;
      let fill;
      if (tab.value === "lightness") {
        fill = `hsl(${hue} 70% ${Math.round((1 - down) * 100)}%)`;
      } else if (tab.value === "chroma") {
        fill = `hsl(${hue} ${Math.round((1 - down) * 90)}% 55%)`;
      } else {
        const shifted = (hue + (0.5 - down) * 360 + 360) % 360;
        fill = `hsl(${shifted} 75% 55%)`;
      }
      cells.push({ x: (col / GRID_X) * W, y: (row / GRID_Y) * H, w: W / GRID_X + 0.5, h: H / GRID_Y + 0.5, fill });
    }
  }
  return cells;
});

function curvePath() {
  const steps = 120;
  const parts = [];
  for (let i = 0; i <= steps; i += 1) {
    const hue = (i / steps) * 360;
    const x = toX(hue);
    const y = toY(evaluate(points.value, hue));
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
  const span = Math.max(1e-3, x1 - x0);
  const t = Math.max(0, Math.min(1, (hue - x0) / span));
  const linear = y0 + (y1 - y0) * t;
  if (props.node.params?.interpolation === "linear") {
    return linear;
  }
  // One slope per control point, shared by both neighbours, so the curve stays
  // differentiable at the point (per-segment tangents bend it into a corner).
  const m0 = slope(list, index);
  const m1 = slope(list, (index + 1) % n);
  const t2 = t * t;
  const t3 = t2 * t;
  const spline =
    (2 * t3 - 3 * t2 + 1) * y0 +
    (t3 - 2 * t2 + t) * span * m0 +
    (-2 * t3 + 3 * t2) * y1 +
    (t3 - t2) * span * m1;
  const weight = Math.max(0, Math.min(100, Number(props.node.params?.smoothing ?? 30))) / 100;
  return linear + (spline - linear) * weight;
}

function slope(list, index) {
  const n = list.length;
  const y = (i) => list[((i % n) + n) % n][1];
  const xPrev = index === 0 ? list[n - 1][0] - 360 : list[index - 1][0];
  const xNext = index + 1 < n ? list[index + 1][0] : list[0][0] + 360;
  const hPrev = Math.max(1e-3, list[index][0] - xPrev);
  const hNext = Math.max(1e-3, xNext - list[index][0]);
  const prev = y(index - 1);
  const next = y(index + 1);
  const here = list[index][1];
  const sPrev = (here - prev) / hPrev;
  const sNext = (next - here) / hNext;
  if (props.node.params?.interpolation !== "centripetal_spline") {
    return (next - prev) / (hPrev + hNext);
  }
  return (sPrev * hNext + sNext * hPrev) / (hPrev + hNext);
}

function commit(next) {
  const max = activeTab.value.max;
  const clamped = next
    .map(([hue, value]) => [
      Math.max(0, Math.min(360, Math.round(hue))),
      Math.max(-max, Math.min(max, Math.round(value))),
    ])
    .sort((a, b) => a[0] - b[0]);
  // Two points on the same hue would make a vertical step; keep the last one.
  const merged = [];
  for (const point of clamped) {
    const last = merged[merged.length - 1];
    if (last && Math.abs(last[0] - point[0]) < 1) {
      last[1] = point[1];
    } else {
      merged.push([...point]);
    }
  }
  if (merged.length >= 2) {
    const first = merged[0];
    const last = merged[merged.length - 1];
    if (first[0] <= 1 && last[0] >= 359) {
      last[1] = first[1];
    }
  }
  props.node.params[activeTab.value.key] = merged;
  emit("pick");
}

function localPoint(event) {
  const rect = svg.value.getBoundingClientRect();
  return {
    x: ((event.clientX - rect.left) / rect.width) * W,
    y: ((event.clientY - rect.top) / rect.height) * H,
    rect,
  };
}

function nearestIndex(event) {
  if (!svg.value) return -1;
  const { x, y, rect } = localPoint(event);
  const scaleX = W / rect.width;
  const scaleY = H / rect.height;
  let best = -1;
  let bestDistance = HIT_PX;
  points.value.forEach(([hue, value], index) => {
    const dx = (toX(hue) - x) / scaleX;
    const dy = (toY(value) - y) / scaleY;
    const distance = Math.hypot(dx, dy);
    if (distance <= bestDistance) {
      bestDistance = distance;
      best = index;
    }
  });
  return best;
}

// Pressing an existing point drags it; pressing empty space inserts one there and
// keeps dragging it, so a click-drag shapes the curve in one gesture.
function onPointerDown(event) {
  if (event.button !== 0 || !svg.value) return;
  event.preventDefault();
  const { x, y } = localPoint(event);
  let index = nearestIndex(event);
  if (index < 0) {
    // A second point on the same hue would be a vertical step, not a curve.
    const sameHue = points.value.findIndex(([entry]) => Math.abs(entry - x) < 1.5);
    if (sameHue >= 0) index = sameHue;
  }
  if (index < 0) {
    const list = [...points.value.map((point) => [...point])];
    const hue = Math.max(0, Math.min(360, x));
    const insert = list.findIndex(([entry]) => entry > hue);
    const point = [hue, fromY(y)];
    if (insert < 0) list.push(point);
    else list.splice(insert, 0, point);
    props.node.params[activeTab.value.key] = list;
    index = insert < 0 ? list.length - 1 : insert;
  }
  dragIndex = index;
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
}

function onPointerMove(event) {
  if (dragIndex < 0 || !svg.value) return;
  event.preventDefault();
  const { x, y } = localPoint(event);
  const list = points.value.map((entry) => [...entry]);
  const lower = dragIndex > 0 ? list[dragIndex - 1][0] + 1 : 0;
  const upper = dragIndex < list.length - 1 ? list[dragIndex + 1][0] - 1 : 360;
  list[dragIndex][0] = Math.max(lower, Math.min(upper, x));
  list[dragIndex][1] = fromY(y);
  // 0° and 360° are the same hue: keep the two ends on one value.
  if (list.length > 2) {
    if (dragIndex === 0 && list[list.length - 1][0] >= 359) {
      list[list.length - 1][1] = list[0][1];
    } else if (dragIndex === list.length - 1 && list[0][0] <= 1) {
      list[0][1] = list[list.length - 1][1];
    }
  }
  props.node.params[activeTab.value.key] = list;
}

function onPointerUp() {
  window.removeEventListener("pointermove", onPointerMove);
  if (dragIndex < 0) return;
  dragIndex = -1;
  commit(points.value);
}

function removePoint(event, index) {
  event.preventDefault();
  event.stopPropagation();
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
      <svg
        ref="svg"
        class="relative h-32 w-full cursor-crosshair touch-none"
        :viewBox="`0 0 ${W} ${H}`"
        preserveAspectRatio="none"
        @pointerdown="onPointerDown"
      >
        <rect
          v-for="(cell, index) in background"
          :key="index"
          :x="cell.x"
          :y="cell.y"
          :width="cell.w"
          :height="cell.h"
          :fill="cell.fill"
          opacity="0.72"
        />
        <line :x1="0" :x2="W" :y1="H / 2" :y2="H / 2" :stroke="isDark ? 'rgba(255,255,255,0.5)' : 'rgba(0,0,0,0.35)'" stroke-width="0.7" />
        <path :d="curvePath()" fill="none" :stroke="isDark ? '#fbbf24' : '#b45309'" stroke-width="1.6" />
        <g v-for="([hue, value], index) in points" :key="`${hue}:${value}:${index}`">
          <circle
            :cx="toX(hue)"
            :cy="toY(value)"
            r="9"
            fill="transparent"
            class="cursor-grab"
            @pointerdown.stop="onPointerDown($event)"
            @dblclick.stop="removePoint($event, index)"
            @contextmenu.prevent.stop="removePoint($event, index)"
          />
          <circle
            :cx="toX(hue)"
            :cy="toY(value)"
            r="4"
            :fill="isDark ? '#fde68a' : '#92400e'"
            :stroke="isDark ? '#78350f' : '#fff7ed'"
            stroke-width="1"
            class="pointer-events-none"
          />
        </g>
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
