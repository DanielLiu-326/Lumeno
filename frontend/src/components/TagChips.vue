<script setup>
import { computed, nextTick, ref } from "vue";
import { t } from "../i18n/index.js";
import { IconClose, IconPlus } from "../utils/icons";

const props = defineProps({
  modelValue: { type: Array, default: () => [] },
  available: { type: Array, default: () => [] },
  editable: { type: Boolean, default: false },
  isDark: { type: Boolean, default: false },
  variant: { type: String, default: "" },
});

const emit = defineEmits(["update:modelValue"]);

const draft = ref("");
const adding = ref(false);
const inputRef = ref(null);

const tags = computed(() =>
  (props.modelValue || []).map((tag) => String(tag).trim()).filter(Boolean),
);

const SUGGESTION_LIMIT = 24;

const suggestions = computed(() => {
  const have = new Set(tags.value.map((tag) => tag.toLowerCase()));
  const query = draft.value.trim().toLowerCase();
  const matched = [];
  for (const tag of props.available || []) {
    const value = String(tag || "").trim();
    if (!value || have.has(value.toLowerCase())) continue;
    if (query && !value.toLowerCase().includes(query)) continue;
    matched.push(value);
    if (matched.length >= SUGGESTION_LIMIT) break;
  }
  return matched;
});

const chipClass = computed(() => [
  props.isDark ? "is-dark" : "",
  props.variant === "filter" ? "is-filter" : "",
]);

function setTags(next) {
  emit("update:modelValue", next);
}

function addTag(raw) {
  const value = String(raw || "").trim();
  if (!value) return;
  if (tags.value.some((tag) => tag.toLowerCase() === value.toLowerCase())) return;
  setTags([...tags.value, value]);
}

function removeTag(tag) {
  if (!props.editable) return;
  setTags(tags.value.filter((item) => item !== tag));
}

function startAdd() {
  adding.value = true;
  nextTick(() => inputRef.value?.focus());
}

function commitDraft() {
  addTag(draft.value);
  draft.value = "";
}

function finishAdd() {
  commitDraft();
  adding.value = false;
}

function onDraftKey(event) {
  if (event.key === "Enter" || event.key === ",") {
    event.preventDefault();
    commitDraft();
  } else if (event.key === "Escape") {
    event.preventDefault();
    draft.value = "";
    adding.value = false;
  }
}

function labelFor(tag) {
  return props.variant === "filter" ? `#${tag}` : tag;
}

const chipItems = computed(() => {
  const selected = tags.value.map((tag) => ({ tag, selected: true }));
  if (!props.editable) return selected;
  const suggested = suggestions.value.map((tag) => ({ tag, selected: false }));
  return [...selected, ...suggested];
});

function onChip(item) {
  if (!props.editable) return;
  if (item.selected) removeTag(item.tag);
  else addTag(item.tag);
}
</script>

<template>
  <TransitionGroup
    tag="div"
    name="tag-chip"
    class="tag-chips"
    :class="editable ? 'is-editing' : ''"
  >
    <component
      :is="editable ? 'button' : 'span'"
      v-for="item in chipItems"
      :key="item.tag"
      :type="editable ? 'button' : undefined"
      class="tag-chip"
      :class="[chipClass, editable ? (item.selected ? 'is-on' : 'is-suggest') : '']"
      :aria-pressed="editable ? String(item.selected) : undefined"
      @click="onChip(item)"
    >
      <span v-if="editable" class="tag-chip-plus" :class="item.selected ? 'is-off' : ''" aria-hidden="true">+</span>
      {{ labelFor(item.tag) }}
      <span
        v-if="editable"
        class="tag-chip-remove"
        :class="item.selected ? '' : 'is-off'"
        aria-hidden="true"
      >
        <IconClose class="h-3 w-3" />
      </span>
    </component>

    <div v-if="editable && adding" key="tag-input" class="tag-chip is-input" :class="chipClass">
      <input
        ref="inputRef"
        v-model="draft"
        type="text"
        class="tag-chip-field"
        :placeholder="t('detail.addTag')"
        autocomplete="off"
        @keydown="onDraftKey"
        @blur="finishAdd"
      />
    </div>
    <button
      v-else-if="editable"
      key="tag-add"
      type="button"
      class="tag-chip is-add"
      :class="chipClass"
      @click="startAdd"
    >
      <IconPlus class="h-3.5 w-3.5" />
      {{ t("detail.addTag") }}
    </button>
    <span v-else-if="tags.length === 0" key="tag-empty" class="tag-chip-empty">{{ t("mobile.noTags") }}</span>
  </TransitionGroup>
</template>
