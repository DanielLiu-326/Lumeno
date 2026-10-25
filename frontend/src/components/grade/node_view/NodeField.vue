<script setup>
import AppSlider from "../../AppSlider.vue";

defineProps({
  node: { type: Object, required: true },
  field: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["slider", "choice", "toggle"]);

function sliderStep(field) {
  const span = Number(field.max) - Number(field.min);
  if (span >= 1000) return 10;
  if (span >= 20) return 1;
  if (span >= 2) return 0.01;
  return 0.001;
}

function readout(field) {
  if (field.kind !== "slider") return field.display;
  const value = Number(field.value);
  if (!Number.isFinite(value)) return field.display;
  if (Math.abs(field.max - field.min) >= 20) return value.toFixed(0);
  return value.toFixed(2);
}
</script>

<template>
  <div class="flex items-center gap-2 py-1">
    <span class="min-w-16 shrink-0 text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ field.label }}</span>
    <template v-if="field.kind === 'readonly'">
      <span class="min-w-0 flex-1 truncate text-[11px]" :class="isDark ? 'text-stone-200' : 'text-stone-800'">{{ field.display }}</span>
    </template>
    <template v-else-if="field.kind === 'slider'">
      <div class="min-w-0 flex-1">
        <AppSlider
          :model-value="field.value"
          :min="field.min"
          :max="field.max"
          :step="sliderStep(field)"
          tone="ghost"
          wheel
          :is-dark="isDark"
          :aria-label="field.label"
          @input="emit('slider', { field, value: $event })"
        />
      </div>
      <span class="w-12 shrink-0 text-right text-[11px] tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ readout(field) }}</span>
    </template>
    <template v-else-if="field.kind === 'choice'">
      <div class="flex min-w-0 flex-1 flex-wrap gap-1">
        <button v-for="(option, index) in field.options" :key="option.value" type="button"
          class="rounded px-1.5 py-0.5 text-[11px]"
          :class="Number(field.value) === index
            ? (isDark ? 'bg-white/15 text-stone-50' : 'bg-stone-900 text-white')
            : (isDark ? 'bg-white/8 text-stone-300 hover:bg-white/12' : 'bg-white/80 text-stone-600 hover:bg-white')"
          @click="emit('choice', { field, option, index })">
          {{ option.label }}
        </button>
      </div>
    </template>
    <template v-else-if="field.kind === 'toggle'">
      <label class="app-switch ml-auto">
        <input type="checkbox" :checked="node.params[field.key] !== false && field.value >= 0.5"
          @change="emit('toggle', { field, on: $event.target.checked })" />
        <span class="app-switch-track"></span>
      </label>
    </template>
  </div>
</template>
