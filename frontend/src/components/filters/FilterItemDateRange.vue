<script setup>
import TimelineChart from "../TimelineChart.vue";
import { t } from "../../i18n/index.js";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark:     { type: Boolean, default: false },
  avail:      { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

function set(patch) {
  emit("update:modelValue", { ...props.modelValue, ...patch });
  emit("apply");
}
function setMode(mode) {
  // Only persist mode switch, do not trigger a re-filter
  emit("update:modelValue", { ...props.modelValue, mode });
}
</script>

<template>
  <div class="min-w-0 max-w-full overflow-hidden">
    <!-- Mode toggle: picker / timeline -->
    <div
      class="mb-1.5 flex rounded-md border text-[11px]"
      :class="isDark ? 'border-stone-700' : 'border-stone-300'"
    >
      <button
        class="flex-1 px-1.5 py-1 font-medium transition-colors"
        :class="modelValue.mode === 'picker'
          ? (isDark ? 'bg-stone-700 text-stone-100' : 'bg-stone-200 text-stone-800')
          : (isDark ? 'text-stone-500 hover:text-stone-300' : 'text-stone-400 hover:text-stone-600')"
        @click="setMode('picker')"
      >{{ t("albums.datePicker") }}</button>
      <button
        class="flex-1 px-1.5 py-1 font-medium transition-colors"
        :class="modelValue.mode === 'timeline'
          ? (isDark ? 'bg-stone-700 text-stone-100' : 'bg-stone-200 text-stone-800')
          : (isDark ? 'text-stone-500 hover:text-stone-300' : 'text-stone-400 hover:text-stone-600')"
        @click="setMode('timeline')"
      >{{ t("albums.timeline") }}</button>
    </div>

    <!-- Picker mode -->
    <template v-if="modelValue.mode === 'picker'">
      <input
        :value="modelValue.from"
        type="date"
        class="mb-1 box-border min-w-0 max-w-full w-full rounded-lg border px-2 py-1.5 text-xs app-control"
        :class="isDark ? 'text-stone-100' : 'text-stone-900'"
        @change="set({ from: $event.target.value })"
      />
      <input
        :value="modelValue.to"
        type="date"
        class="box-border min-w-0 max-w-full w-full rounded-lg border px-2 py-1.5 text-xs app-control"
        :class="isDark ? 'text-stone-100' : 'text-stone-900'"
        @change="set({ to: $event.target.value })"
      />
    </template>

    <!-- Timeline mode -->
    <template v-else>
      <div class="min-w-0 max-w-full overflow-hidden">
        <TimelineChart
          :buckets="avail.dateHistogram || []"
          :is-dark="isDark"
          :from="modelValue.from"
          :to="modelValue.to"
          @select-range="(from, to) => set({ from, to })"
        />
      </div>
      <p
        v-if="modelValue.from || modelValue.to"
        class="mt-1 text-[11px] leading-tight"
        :class="isDark ? 'text-stone-400' : 'text-stone-500'"
      >
        {{ modelValue.from || "…" }} → {{ modelValue.to || "…" }}
      </p>
    </template>
  </div>
</template>

<style scoped>
input[type="date"] {
  min-inline-size: 0;
}
</style>
