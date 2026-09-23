<script setup>
import { toRef } from "vue";
import { t } from "../../i18n/index.js";
import { IconClose } from "../../utils/icons";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { useSheetDrag } from "../useSheetDrag.js";

const props = defineProps({
  show: { type: Boolean, default: false },
  isDark: { type: Boolean, default: false },
  items: { type: Array, default: () => [] },
});

const emit = defineEmits(["close", "clear"]);
const openRef = toRef(props, "show");
useOverlayHistory(openRef, () => emit("close"));
const { dragging, expanded, panelStyle, maskStyle, onPointerDown } = useSheetDrag(openRef, () => emit("close"));
</script>

<template>
  <Teleport to="body">
    <Transition name="mobile-sheet">
      <div v-if="show" class="mobile-sheet-root">
        <button type="button" class="mobile-sheet-mask" aria-label="Close" :style="maskStyle" @click="emit('close')"></button>
        <div
          class="mobile-sheet-panel mobile-sheet-panel-tall"
          :class="{ 'is-dragging': dragging, 'is-expanded': expanded }"
          :style="panelStyle"
          @pointerdown="onPointerDown"
        >
          <div class="mobile-sheet-handle"></div>
          <div class="flex items-center justify-between px-4 pb-3">
            <p class="text-base font-semibold">{{ t("topbar.notificationCenter") }}</p>
            <div class="flex items-center gap-2">
              <button type="button" class="mobile-chip-btn" @click="emit('clear')">{{ t("topbar.clear") }}</button>
              <button type="button" class="mobile-icon-btn" @click="emit('close')"><IconClose class="h-4 w-4" /></button>
            </div>
          </div>
          <div class="mobile-sheet-scroll">
            <div v-for="item in items" :key="item.id" class="app-split-b px-4 py-3 last:border-b-0">
              <p class="text-sm" :class="item.kind === 'error' ? 'text-rose-500' : ''">{{ item.text }}</p>
              <p class="mt-1 text-xs opacity-50">{{ item.createdAt }}</p>
            </div>
            <div v-if="items.length === 0" class="px-4 py-10 text-center text-sm opacity-60">{{ t("topbar.noNotifications") }}</div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
