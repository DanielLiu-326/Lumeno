<script setup>
import { computed, ref, toRef, watch } from "vue";
import { t } from "../../i18n/index.js";
import { FILTER_DEF_MAP } from "../../utils/filterDefs.js";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { useSheetDrag } from "../useSheetDrag.js";

const props = defineProps({
  show: { type: Boolean, default: false },
  isDark: { type: Boolean, default: false },
  item: { type: Object, default: null },
  avail: { type: Object, default: () => ({}) },
  title: { type: String, default: "" },
});

const emit = defineEmits(["close", "apply"]);

const openRef = toRef(props, "show");
useOverlayHistory(openRef, () => emit("close"));
const { dragging, expanded, panelStyle, maskStyle, onPointerDown } = useSheetDrag(openRef, () => emit("close"));

const ready = ref(false);

watch(
  () => props.show,
  (open) => {
    ready.value = false;
    if (!open) return;
    window.setTimeout(() => {
      ready.value = true;
    }, 320);
  },
);

const editor = computed(() => {
  const type = props.item?.type;
  return type ? FILTER_DEF_MAP[type]?.component : null;
});

function setValue(next) {
  if (!props.item) return;
  props.item.value = next;
}

function close() {
  if (!ready.value) return;
  emit("close");
}
</script>

<template>
  <Teleport to="body">
    <Transition name="mobile-sheet">
      <div v-if="show && item && editor" class="mobile-sheet-root is-stacked">
        <button type="button" class="mobile-sheet-mask" aria-label="Close" :style="maskStyle" @click="close"></button>
        <div
          class="mobile-sheet-panel mobile-sheet-panel-auto"
          :class="{ 'is-dragging': dragging, 'is-expanded': expanded }"
          :style="panelStyle"
          @pointerdown="onPointerDown"
        >
          <div class="mobile-sheet-handle"></div>
          <div class="px-4 pb-2 text-center">
            <p class="truncate text-sm font-semibold">{{ title }}</p>
          </div>
          <div class="mobile-sheet-scroll app-scrollbar mobile-filter-editor px-4 py-2">
            <component
              :is="editor"
              :model-value="item.value || {}"
              :is-dark="isDark"
              :avail="avail"
              @update:model-value="setValue"
              @apply="emit('apply')"
            />
          </div>
          <button type="button" class="mobile-sheet-cancel" @click="close">{{ t("mobile.done") }}</button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.mobile-filter-editor :deep(.max-h-52) {
  max-height: none;
}

.mobile-filter-editor :deep(input:not(.tag-chip-field)),
.mobile-filter-editor :deep(select) {
  min-height: 44px;
  font-size: 16px;
}

.mobile-filter-editor :deep(.h-6.w-6.rounded-full) {
  height: 2.25rem;
  width: 2.25rem;
}

.mobile-filter-editor :deep(.h-5.w-5) {
  height: 1.65rem;
  width: 1.65rem;
}
</style>
