<script setup>
import { computed, ref } from "vue";

const props = defineProps({
  // [{ key, label, value, color }]
  segments: { type: Array, default: () => [] },
  centerLabel: { type: String, default: "" },
  centerValue: { type: String, default: "" },
  size: { type: Number, default: 180 },
  isDark: { type: Boolean, default: false },
  format: { type: Function, default: (value) => String(value) },
  /// Ring on the left, legend in two columns on the right.
  row: { type: Boolean, default: false },
});

const active = ref(null);

const total = computed(() =>
  props.segments.reduce((sum, segment) => sum + (Number(segment.value) || 0), 0),
);

// Only non-zero slices are drawn; percentages are normalised so rounding never
// leaves a gap in the ring.
const drawn = computed(() => {
  if (!total.value) return [];
  let offset = 0;
  return props.segments
    .filter((segment) => Number(segment.value) > 0)
    .map((segment) => {
      const percent = (Number(segment.value) / total.value) * 100;
      const slice = { ...segment, percent, offset };
      offset += percent;
      return slice;
    });
});

const activeSlice = computed(() =>
  drawn.value.find((slice) => slice.key === active.value) || null,
);

const ringTrack = computed(() => (props.isDark ? "rgba(255,255,255,0.12)" : "#e7e5e4"));
function enter(key) {
  active.value = key;
}
function leave() {
  active.value = null;
}
</script>

<template>
  <div class="flex gap-4" :class="row ? 'flex-col items-start sm:flex-row sm:items-center' : 'flex-col items-stretch'">
    <div class="relative shrink-0 self-center" :style="{ width: `${size}px`, height: `${size}px` }">
      <svg
        class="h-full w-full -rotate-90"
        viewBox="0 0 120 120"
        role="img"
        :aria-label="centerLabel"
      >
        <circle
          cx="60"
          cy="60"
          r="46"
          fill="none"
          :stroke="ringTrack"
          stroke-width="14"
          pathLength="100"
        />
        <circle
          v-for="slice in drawn"
          :key="slice.key"
          cx="60"
          cy="60"
          r="46"
          fill="none"
          :stroke="slice.color"
          stroke-width="14"
          pathLength="100"
          :stroke-dasharray="`${slice.percent} ${100 - slice.percent}`"
          :stroke-dashoffset="-slice.offset"
          class="cursor-pointer transition-opacity duration-150"
          :class="active && active !== slice.key ? 'opacity-30' : 'opacity-100'"
          @mouseenter="enter(slice.key)"
          @mouseleave="leave"
        >
          <title>{{ slice.label }} · {{ format(slice.value) }}</title>
        </circle>
      </svg>
      <div class="pointer-events-none absolute inset-0 flex flex-col items-center justify-center text-center">
        <span class="max-w-[70%] truncate text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
          {{ activeSlice ? activeSlice.label : centerLabel }}
        </span>
        <span class="mt-0.5 text-lg font-bold" :class="isDark ? 'text-stone-50' : 'text-stone-900'">
          {{ activeSlice ? format(activeSlice.value) : centerValue }}
        </span>
      </div>
    </div>

    <ul class="w-full min-w-0" :class="row ? 'grid grid-cols-1 gap-x-6 gap-y-1 sm:grid-cols-2' : 'space-y-1'">
      <li
        v-for="slice in drawn"
        :key="slice.key"
        class="flex cursor-pointer items-center gap-2 rounded-md px-1 py-0.5 text-xs transition-colors"
        :class="active === slice.key
          ? isDark ? 'bg-stone-800' : 'bg-stone-100'
          : ''"
        @mouseenter="enter(slice.key)"
        @mouseleave="leave"
      >
        <span class="h-2.5 w-2.5 shrink-0 rounded-full" :style="{ background: slice.color }" />
        <span class="min-w-0 flex-1 truncate" :title="slice.label" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ slice.label }}</span>
        <span class="shrink-0 tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ format(slice.value) }}</span>
        <span class="w-10 shrink-0 text-right tabular-nums" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ slice.percent.toFixed(0) }}%</span>
      </li>
      <li v-if="!drawn.length" class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ centerValue }}
      </li>
    </ul>
  </div>
</template>
