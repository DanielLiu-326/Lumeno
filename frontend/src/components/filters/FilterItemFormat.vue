<script setup>
import { t } from "../../i18n/index.js";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark:     { type: Boolean, default: false },
  avail:      { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

function set(format) {
  emit("update:modelValue", { ...props.modelValue, format });
  emit("apply");
}
</script>

<template>
  <select
    :value="modelValue.format"
    class="w-full rounded-lg border px-2 py-1.5 text-xs"
    :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
    @change="set($event.target.value)"
  >
    <option value="">{{ t("albums.all") }}</option>
    <option v-for="fmt in (avail.availableFormats || [])" :key="fmt" :value="fmt">{{ fmt }}</option>
  </select>
</template>
