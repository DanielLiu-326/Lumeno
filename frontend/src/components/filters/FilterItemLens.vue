<script setup>
import { t } from "../../i18n/index.js";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark:     { type: Boolean, default: false },
  avail:      { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

function set(lens) {
  emit("update:modelValue", { ...props.modelValue, lens });
  emit("apply");
}
</script>

<template>
  <select
    :value="modelValue.lens"
    class="w-full rounded-lg border px-2 py-1.5 text-xs app-control"
    :class="isDark ? 'text-stone-100' : 'text-stone-900'"
    @change="set($event.target.value)"
  >
    <option value="">{{ t("albums.all") }}</option>
    <option v-for="l in (avail.availableLenses || [])" :key="l" :value="l">{{ l }}</option>
  </select>
</template>
