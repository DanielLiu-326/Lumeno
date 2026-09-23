<script setup>
import { computed } from "vue";
import TagChips from "../TagChips.vue";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark:     { type: Boolean, default: false },
  avail:      { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

const selected = computed(() =>
  String(props.modelValue.tags || "")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean),
);

function setTags(next) {
  emit("update:modelValue", {
    ...props.modelValue,
    tags: (next || []).map((tag) => String(tag).trim()).filter(Boolean).join(", "),
  });
  emit("apply");
}
</script>

<template>
  <div class="filter-tags-list app-scrollbar">
    <TagChips
      variant="filter"
      :model-value="selected"
      :available="avail.availableTags || []"
      editable
      :is-dark="isDark"
      @update:model-value="setTags"
    />
  </div>
</template>
