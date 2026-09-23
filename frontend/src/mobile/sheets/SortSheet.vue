<script setup>
import { computed, ref, toRef, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import { t } from "../../i18n/index.js";
import { IconBroom, IconPlusHeavy, IconTrash } from "../../utils/icons";
import {
  SORT_FIELDS,
  withAddedSortField,
  withRemovedSortLevel,
  withToggledSortDir,
} from "../../utils/filterDefs.js";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { useSheetDrag } from "../useSheetDrag.js";
import { useSwipeReveal } from "../useSwipeReveal.js";
import ActionSheet from "./ActionSheet.vue";

const ACTION_WIDTH = 88;

const props = defineProps({
  show: { type: Boolean, default: false },
  isDark: { type: Boolean, default: false },
  sort: { type: Array, default: () => [] },
});

const emit = defineEmits(["close", "update-sort"]);

const openRef = toRef(props, "show");
useOverlayHistory(openRef, () => emit("close"));
const { dragging, expanded, panelStyle, maskStyle, onPointerDown } = useSheetDrag(openRef, () => emit("close"));
const {
  revealId,
  draggingId,
  closeReveal,
  lock,
  unlock,
  rowOffset,
  onSwipeDown,
} = useSwipeReveal(ACTION_WIDTH);

const addOpen = ref(false);
const sortItems = ref([]);

watch(
  () => props.sort,
  (list) => {
    sortItems.value = (list || []).map((level) => ({ ...level }));
  },
  { immediate: true, deep: true },
);

const addActions = computed(() => {
  const used = new Set(sortItems.value.map((s) => s.field));
  return SORT_FIELDS.filter((f) => !used.has(f.key)).map((f) => ({
    id: f.key,
    label: t(f.labelKey),
  }));
});

watch(
  () => props.show,
  (open) => {
    if (open) return;
    addOpen.value = false;
    closeReveal();
  },
);

function fieldLabel(field) {
  const def = SORT_FIELDS.find((f) => f.key === field);
  return def ? t(def.labelKey) : field;
}

function commit(next = sortItems.value) {
  emit("update-sort", next.map((level) => ({ ...level })));
}

function toggleDir(index) {
  commit(withToggledSortDir(sortItems.value, index));
}

function removeLevel(index) {
  commit(withRemovedSortLevel(sortItems.value, index));
  closeReveal();
}

function pickField(action) {
  addOpen.value = false;
  commit(withAddedSortField(sortItems.value, action.id));
}

function clearSort() {
  commit([]);
  closeReveal();
}

function handleDragStart() {
  lock();
}

function handleDragEnd() {
  unlock();
}
</script>

<template>
  <Teleport to="body">
    <Transition name="mobile-sheet">
      <div v-if="show" class="mobile-sheet-root">
        <button type="button" class="mobile-sheet-mask" aria-label="Close" :style="maskStyle" @click="emit('close')"></button>
        <div
          class="mobile-sheet-panel mobile-sheet-panel-half"
          :class="{ 'is-dragging': dragging, 'is-expanded': expanded }"
          :style="panelStyle"
          @pointerdown="onPointerDown"
        >
          <div class="mobile-sheet-handle"></div>
          <div class="flex items-center gap-2 px-4 pb-2">
            <p class="min-w-0 flex-1 truncate text-base font-semibold">{{ t("albums.sort") }}</p>
            <button type="button" class="mobile-icon-btn" :title="t('albums.clear')" @click="clearSort">
              <IconBroom class="h-6 w-6" />
            </button>
            <button
              v-if="addActions.length > 0"
              type="button"
              class="mobile-icon-btn"
              :class="addOpen ? 'is-accent' : ''"
              :title="t('albums.addSortLevel')"
              @click="addOpen = true"
            >
              <IconPlusHeavy class="h-6 w-6" />
            </button>
          </div>
          <div class="mobile-sheet-scroll app-scrollbar flex flex-col pb-3">
            <p v-if="sortItems.length === 0" class="px-4 py-6 text-center text-sm opacity-50">
              {{ t("albums.sortHint") }}
            </p>
            <VueDraggable
              v-model="sortItems"
              class="flex flex-col"
              :animation="220"
              direction="vertical"
              ghost-class="filter-sortable-ghost"
              drag-class="filter-sortable-drag"
              chosen-class="filter-sortable-chosen"
              :force-fallback="true"
              fallback-class="filter-sortable-drag"
              :delay="500"
              :delay-on-touch-only="true"
              :touch-start-threshold="16"
              :fallback-tolerance="16"
              filter=".mobile-sort-dir-chip, .mobile-filter-action, .mobile-filter-actions"
              :prevent-on-filter="false"
              @start="handleDragStart"
              @end="handleDragEnd"
              @update="commit"
            >
              <div
                v-for="(item, idx) in sortItems"
                :key="item.field"
                class="mobile-filter-swipe"
                :class="{
                  'is-open': revealId === item.field,
                  'is-revealing': draggingId === item.field,
                }"
              >
                <div class="mobile-filter-actions is-single" @pointerdown.stop>
                  <button type="button" class="mobile-filter-action is-delete" @click="removeLevel(idx)">
                    <IconTrash class="h-4 w-4" />
                    {{ t("filter.delete") }}
                  </button>
                </div>
                <div
                  class="mobile-filter-front"
                  :class="{ 'is-busy': draggingId === item.field }"
                  :style="{ transform: `translate3d(${rowOffset(item.field)}px,0,0)` }"
                >
                  <div
                    class="mobile-filter-head"
                    @pointerdown="onSwipeDown($event, item.field, { ignore: '.mobile-sort-dir-chip' })"
                  >
                    <span class="min-w-0 flex-1 truncate text-left text-[15px] font-medium">
                      {{ fieldLabel(item.field) }}
                    </span>
                    <button
                      type="button"
                      class="mobile-sort-dir-chip"
                      @pointerdown.stop
                      @click.stop="toggleDir(idx)"
                    >
                      {{ item.dir === "asc" ? t("albums.sortAsc") : t("albums.sortDesc") }}
                    </button>
                  </div>
                </div>
              </div>
            </VueDraggable>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
  <ActionSheet
    :show="show && addOpen"
    :title="t('albums.addSortField')"
    :actions="addActions"
    @close="addOpen = false"
    @select="pickField"
  />
</template>

<style scoped>
:deep(.filter-sortable-ghost) {
  opacity: 0;
  border-radius: 1rem;
}

:deep(.filter-sortable-drag) {
  opacity: 0.55;
  border-radius: 1rem;
}

:deep(.filter-sortable-chosen) {
  cursor: grabbing;
  opacity: 0.55;
}
</style>
