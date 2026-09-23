<script setup>
import { computed, toRef } from "vue";
import { t } from "../../i18n/index.js";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { useSheetDrag } from "../useSheetDrag.js";

const props = defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: "" },
  subtitle: { type: String, default: "" },
  actions: { type: Array, default: () => [] },
  stacked: { type: Boolean, default: false },
});

const emit = defineEmits(["close", "select"]);

const openRef = toRef(props, "show");
useOverlayHistory(openRef, () => emit("close"));
const { dragging, expanded, panelStyle, maskStyle, onPointerDown } = useSheetDrag(openRef, () => emit("close"));

const cancelLabel = computed(() => t("mobile.cancel"));

function pick(action) {
  if (action?.disabled) return;
  emit("select", action);
}
</script>

<template>
  <Teleport to="body">
    <Transition name="mobile-sheet">
      <div v-if="show" class="mobile-sheet-root" :class="{ 'is-stacked': stacked }">
        <button type="button" class="mobile-sheet-mask" aria-label="Close" :style="maskStyle" @click="emit('close')"></button>
        <div
          class="mobile-sheet-panel"
          :class="{ 'is-dragging': dragging, 'is-expanded': expanded }"
          :style="panelStyle"
          @pointerdown="onPointerDown"
        >
          <div class="mobile-sheet-handle"></div>
          <div v-if="title || subtitle" class="px-4 pb-2 text-center">
            <p v-if="title" class="truncate text-sm font-semibold">{{ title }}</p>
            <p v-if="subtitle" class="mt-0.5 truncate text-xs opacity-60">{{ subtitle }}</p>
          </div>
          <div class="mobile-sheet-scroll flex flex-col gap-1 px-3">
            <slot :pick="pick">
              <button
                v-for="action in actions"
                :key="action.id"
                type="button"
                class="mobile-sheet-action"
                :class="action.danger ? 'is-danger' : ''"
                :disabled="action.disabled"
                @click="pick(action)"
              >
                <component :is="action.icon" v-if="action.icon" class="h-5 w-5 shrink-0 opacity-80" />
                <span class="min-w-0 flex-1 truncate text-left">{{ action.label }}</span>
              </button>
            </slot>
          </div>
          <button type="button" class="mobile-sheet-cancel" @click="emit('close')">{{ cancelLabel }}</button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
