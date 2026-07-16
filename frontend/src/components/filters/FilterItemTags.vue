<script setup>
import { t } from "../../i18n/index.js";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark:     { type: Boolean, default: false },
  avail:      { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

// Update stored value without triggering a re-filter (wait for Enter)
function onInput(tags) {
  emit("update:modelValue", { ...props.modelValue, tags });
}

// Add a tag chip and immediately re-filter
function addTag(tag) {
  const existing = (props.modelValue.tags || "")
    .split(",").map((s) => s.trim()).filter(Boolean);
  if (!existing.includes(tag)) existing.push(tag);
  emit("update:modelValue", { ...props.modelValue, tags: existing.join(", ") });
  emit("apply");
}
</script>

<template>
  <div>
    <input
      :value="modelValue.tags"
      type="text"
      :placeholder="t('albums.tagsSeparator')"
      class="mb-1.5 w-full rounded-lg border px-2 py-1.5 text-xs"
      :class="isDark
        ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500'
        : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
      @input="onInput($event.target.value)"
      @keyup.enter="emit('apply')"
    />
    <div v-if="(avail.availableTags || []).length > 0" class="flex flex-wrap gap-1">
      <button
        v-for="tag in (avail.availableTags || [])"
        :key="tag"
        class="rounded-full px-2 py-0.5 text-[10px] font-medium"
        :class="isDark ? 'bg-sky-900/60 text-sky-100 hover:bg-sky-800' : 'bg-sky-100 text-sky-700 hover:bg-sky-200'"
        @click="addTag(tag)"
      >
        #{{ tag }}
      </button>
    </div>
  </div>
</template>
