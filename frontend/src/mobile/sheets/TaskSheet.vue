<script setup>
import { toRef } from "vue";
import { t } from "../../i18n/index.js";
import { progressPercent, statusLabel, statusTone } from "../../utils/format";
import { IconClear, IconClose } from "../../utils/icons";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { useSheetDrag } from "../useSheetDrag.js";

const props = defineProps({
  show: { type: Boolean, default: false },
  isDark: { type: Boolean, default: false },
  tasks: { type: Array, default: () => [] },
});

const emit = defineEmits(["close", "pause", "resume", "cancel", "clear-finished"]);
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
            <p class="text-base font-semibold">{{ t("topbar.tasks") }}</p>
            <div class="flex items-center gap-2">
              <button
                type="button"
                class="mobile-icon-btn"
                :title="t('topbar.clearFinished')"
                :aria-label="t('topbar.clearFinished')"
                @click="emit('clear-finished')"
              >
                <IconClear class="h-4 w-4" />
              </button>
              <button type="button" class="mobile-icon-btn" @click="emit('close')"><IconClose class="h-4 w-4" /></button>
            </div>
          </div>
          <div class="mobile-sheet-scroll">
            <div v-for="task in tasks" :key="task.id" class="app-split-b px-4 py-3 last:border-b-0">
              <div class="flex items-start justify-between gap-3">
                <p class="min-w-0 truncate text-sm font-medium">{{ task.name }}</p>
                <div class="flex shrink-0 items-center gap-2">
                  <button
                    v-if="task.pausable && task.status === 1"
                    type="button"
                    class="rounded-full px-2.5 py-1 text-[11px] font-medium"
                    :class="isDark ? 'bg-amber-900/50 text-amber-300' : 'bg-amber-100 text-amber-800'"
                    @click="emit('pause', task.id)"
                  >{{ t("tasks.pause") }}</button>
                  <button
                    v-if="task.pausable && task.status === 2"
                    type="button"
                    class="rounded-full px-2.5 py-1 text-[11px] font-medium"
                    :class="isDark ? 'bg-emerald-900/50 text-emerald-300' : 'bg-emerald-100 text-emerald-800'"
                    @click="emit('resume', task.id)"
                  >{{ t("tasks.resume") }}</button>
                  <button
                    v-if="task.cancellable && (task.status === 0 || task.status === 1 || task.status === 2)"
                    type="button"
                    class="rounded-full px-2.5 py-1 text-[11px] font-medium"
                    :class="isDark ? 'bg-rose-900/50 text-rose-300' : 'bg-rose-100 text-rose-700'"
                    @click="emit('cancel', task.id)"
                  >{{ t("tasks.cancel") }}</button>
                  <span class="rounded-full px-2.5 py-1 text-[11px] font-medium" :class="statusTone(task.status)">{{ statusLabel(task.status) }}</span>
                </div>
              </div>
              <div class="mt-1 flex items-center justify-between gap-3">
                <p class="min-w-0 truncate text-xs opacity-60">{{ task.current_task || t("topbar.waitingForUpdate") }}</p>
                <span class="shrink-0 text-xs tabular-nums">{{ progressPercent(task) }}%</span>
              </div>
              <div class="mt-2 h-1.5 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-700' : 'bg-stone-200'">
                <div class="h-full rounded-full bg-linear-to-r from-amber-600 via-orange-400 to-emerald-600" :style="{ width: `${progressPercent(task)}%` }"></div>
              </div>
            </div>
            <div v-if="tasks.length === 0" class="px-4 py-10 text-center text-sm opacity-60">{{ t("topbar.noTasks") }}</div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
