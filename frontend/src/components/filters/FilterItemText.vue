<script setup>
import { t } from "../../i18n/index.js";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
  avail: { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

function onInput(q) {
  emit("update:modelValue", { ...props.modelValue, q });
}

function onMinScore(minScore) {
  emit("update:modelValue", { ...props.modelValue, minScore: Number(minScore) || 0 });
}

function onApply() {
  emit("apply");
}
</script>

<template>
  <div class="flex flex-col gap-1.5">
    <input
      :value="modelValue.q || ''"
      type="search"
      :placeholder="t('filter.textPlaceholder')"
      class="w-full rounded-lg border px-2 py-1.5 text-xs"
      :class="isDark
        ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500'
        : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
      @input="onInput($event.target.value)"
      @change="onApply"
      @keyup.enter="onApply"
    />
    <label class="flex flex-col gap-1">
      <span class="flex items-center justify-between text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        <span>{{ t("albums.aiMinScore") }}</span>
        <span class="tabular-nums">{{ Number(modelValue.minScore ?? 0).toFixed(2) }}</span>
      </span>
      <input
        :value="Number(modelValue.minScore ?? 0)"
        type="range"
        min="0"
        max="1"
        step="0.01"
        class="w-full accent-amber-500"
        @input="onMinScore($event.target.value)"
        @change="onApply"
      />
      <span class="text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t("albums.aiMinScoreHint") }}</span>
    </label>
    <p class="text-[10px] leading-relaxed" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
      {{ t("filter.textHint") }}
    </p>
  </div>
</template>
