<script setup>
import { nextTick, ref, watch } from "vue";
import { prettyPath } from "../utils/path";
import { t } from "../i18n/index.js";
import { IconChevronLeft, IconChevronRight } from "../utils/icons";

const props = defineProps({
  currentPath: { type: String, default: "" },
  canGoBack: { type: Boolean, default: false },
  canGoForward: { type: Boolean, default: false },
  busy: { type: Boolean, default: false },
  compact: { type: Boolean, default: false },
});

const emit = defineEmits(["back", "forward", "navigate"]);

const editing = ref(false);
const draft = ref("");
const inputRef = ref(null);

const shownPath = () => prettyPath(props.currentPath);

watch(() => props.currentPath, () => {
  editing.value = false;
});

function startEdit() {
  if (editing.value) return;
  draft.value = shownPath();
  editing.value = true;
  nextTick(() => {
    inputRef.value?.focus();
    inputRef.value?.select();
  });
}

function submit() {
  if (!editing.value) return;
  editing.value = false;
  emit("navigate", draft.value);
}

function cancel() {
  editing.value = false;
}

function onKeydown(event) {
  if (event.key === "Enter") {
    event.preventDefault();
    submit();
  } else if (event.key === "Escape") {
    event.preventDefault();
    cancel();
  }
}
</script>

<template>
  <div class="file-path-bar" :class="{ 'is-compact': compact }">
    <button
      type="button"
      class="file-path-nav"
      :disabled="!canGoBack || busy"
      :title="t('files.back')"
      :aria-label="t('files.back')"
      @click="emit('back')"
    >
      <IconChevronLeft class="h-5 w-5" />
    </button>
    <button
      type="button"
      class="file-path-nav"
      :disabled="!canGoForward || busy"
      :title="t('files.forward')"
      :aria-label="t('files.forward')"
      @click="emit('forward')"
    >
      <IconChevronRight class="h-5 w-5" />
    </button>
    <div class="file-path-field" @click="startEdit">
      <input
        v-if="editing"
        ref="inputRef"
        v-model="draft"
        class="file-path-input"
        type="text"
        :placeholder="t('files.pathPlaceholder')"
        autocomplete="off"
        autocorrect="off"
        spellcheck="false"
        enterkeyhint="go"
        @keydown="onKeydown"
        @blur="submit"
      />
      <span v-else class="file-path-text" :title="shownPath()">{{ shownPath() }}</span>
    </div>
  </div>
</template>
