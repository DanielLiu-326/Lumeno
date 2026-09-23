<script setup>
import { computed, ref, toRef, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import { t } from "../../i18n/index.js";
import {
  IconBroom,
  IconPlusHeavy,
  IconRefresh,
  IconTrash,
  IconChevronRight,
} from "../../utils/icons";
import {
  COLOR_SWATCH,
  LOGIC_LABELS,
  LOGIC_TITLE,
  filterTypeName,
  filterValueSummary,
  useAlbumFilters,
} from "../../composables/useAlbumFilters.js";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { useSheetDrag } from "../useSheetDrag.js";
import { useSwipeReveal } from "../useSwipeReveal.js";
import FilterCollections from "../../components/FilterCollections.vue";
import ActionSheet from "./ActionSheet.vue";
import FilterEditorSheet from "./FilterEditorSheet.vue";

const ACTION_WIDTH = 148;

const props = defineProps({
  show: { type: Boolean, default: false },
  isDark: { type: Boolean, default: false },
  collections: { type: Array, default: () => [] },
  selectedCollectionId: { type: String, default: "all" },
  availableTags: { type: Array, default: () => [] },
  availableCameras: { type: Array, default: () => [] },
  availableLenses: { type: Array, default: () => [] },
  availableFormats: { type: Array, default: () => [] },
  dateHistogram: { type: Array, default: () => [] },
  filters: { type: Object, required: true },
  persistFilters: { type: Boolean, default: true },
  availablePeople: { type: Array, default: () => [] },
  currentUser: { type: String, default: "" },
  currentAlbum: { type: String, default: "" },
});

const emit = defineEmits(["close", "apply", "select-collection"]);

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

const {
  FILTER_REGISTRY,
  filterItems,
  avail,
  selectedGroupId,
  currentGroup,
  treeExpanded,
  addFilter,
  deleteFilter,
  resetFilter,
  resetAllFilters,
  setFilterEnabled,
  cycleLogic,
  onFilterReorder,
  onFilterDragStart,
  applyFilters,
  selectCollectionNode,
  logicClass,
} = useAlbumFilters(props, emit);

const addOpen = ref(false);
const editingId = ref(null);

const editingItem = computed(() =>
  filterItems.value.find((item) => item.id === editingId.value) || null,
);

const addActions = computed(() =>
  FILTER_REGISTRY.map((ft) => ({
    id: ft.key,
    label: filterTypeName(ft.key),
  })),
);

watch(
  () => props.show,
  (open) => {
    if (open) return;
    closeReveal();
    addOpen.value = false;
    editingId.value = null;
  },
);

watch(
  () => props.currentAlbum,
  () => {
    closeReveal();
    addOpen.value = false;
    editingId.value = null;
  },
);

watch(editingItem, (item) => {
  if (editingId.value && !item) editingId.value = null;
});

function openEditor(item) {
  closeReveal();
  addOpen.value = false;
  editingId.value = item.id;
}

function closeEditor() {
  editingId.value = null;
}

function toggleEnabled(item) {
  setFilterEnabled(item, !item.enabled);
}

function handleAdd(action) {
  addOpen.value = false;
  const item = addFilter(action.id);
  if (item?.id) editingId.value = item.id;
}

function handleReset(item) {
  resetFilter(item);
  closeReveal();
}

function handleDelete(id) {
  if (editingId.value === id) editingId.value = null;
  deleteFilter(id);
  closeReveal();
}

function handleResetAll() {
  editingId.value = null;
  resetAllFilters();
}

function handleDragStart(event) {
  lock();
  onFilterDragStart(event);
}

function handleDragEnd() {
  unlock();
  onFilterReorder();
}

function rowSummary(item) {
  return filterValueSummary(item, avail.value);
}

function colorKeys(item) {
  return Array.isArray(item.value?.colorLabels) ? item.value.colorLabels : [];
}
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
          <div class="flex items-center gap-2 px-4 pb-2">
            <p class="min-w-0 flex-1 truncate text-base font-semibold">{{ t("albums.filters") }}</p>
            <button
              type="button"
              class="mobile-icon-btn"
              :title="t('filter.resetAll')"
              @click="handleResetAll"
            >
              <IconBroom class="h-6 w-6" />
            </button>
            <button
              type="button"
              class="mobile-icon-btn"
              :class="addOpen ? 'is-accent' : ''"
              :title="t('filter.add')"
              @click="addOpen = true"
            >
              <IconPlusHeavy class="h-6 w-6" />
            </button>
          </div>

          <div
            v-if="collections.length > 0"
            class="mobile-filter-collections"
          >
            <FilterCollections
              :is-dark="isDark"
              :collections="collections"
              :selected-collection-id="selectedCollectionId"
              :selected-group-id="selectedGroupId"
              :current-group="currentGroup"
              :tree-expanded="treeExpanded"
              list-class="max-h-36"
              @update:selected-group-id="selectedGroupId = $event"
              @select-node="selectCollectionNode"
            />
          </div>

          <div class="mobile-sheet-scroll app-scrollbar flex flex-col pb-3">
            <VueDraggable
              v-model="filterItems"
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
              filter=".app-switch, .mobile-filter-logic, .mobile-filter-action, .mobile-filter-actions"
              :prevent-on-filter="false"
              @start="handleDragStart"
              @end="handleDragEnd"
              @update="onFilterReorder"
            >
              <div
                v-for="item in filterItems"
                :key="item.id"
                class="mobile-filter-swipe"
                :class="{
                  'is-off': !item.enabled,
                  'is-open': revealId === item.id,
                  'is-revealing': draggingId === item.id,
                }"
              >
                <div class="mobile-filter-actions" @pointerdown.stop>
                  <button type="button" class="mobile-filter-action is-reset" @click="handleReset(item)">
                    <IconRefresh class="h-4 w-4" />
                    {{ t("filter.reset") }}
                  </button>
                  <button type="button" class="mobile-filter-action is-delete" @click="handleDelete(item.id)">
                    <IconTrash class="h-4 w-4" />
                    {{ t("filter.delete") }}
                  </button>
                </div>
                <div
                  class="mobile-filter-front"
                  :class="{ 'is-busy': draggingId === item.id }"
                  :style="{ transform: `translate3d(${rowOffset(item.id)}px,0,0)` }"
                >
                  <div
                    class="mobile-filter-head"
                    @pointerdown="onSwipeDown($event, item.id, { ignore: '.app-switch, .mobile-filter-logic', onTap: () => openEditor(item) })"
                  >
                    <button
                      type="button"
                      class="mobile-filter-logic"
                      :class="logicClass(item.logic, isDark)"
                      :title="LOGIC_TITLE[item.logic]"
                      @pointerdown.stop
                      @click.stop="cycleLogic(item)"
                    >
                      {{ LOGIC_LABELS[item.logic] }}
                    </button>
                    <div class="min-w-0 flex-1 text-left">
                      <p class="truncate text-[15px] font-medium">{{ filterTypeName(item.type) }}</p>
                      <div v-if="item.type === 'color_label' && colorKeys(item).length" class="mt-0.5 flex items-center gap-1">
                        <span
                          v-for="key in colorKeys(item)"
                          :key="key"
                          class="h-2.5 w-2.5 rounded-full"
                          :class="COLOR_SWATCH[key]"
                        ></span>
                      </div>
                      <p v-else-if="rowSummary(item)" class="mt-0.5 truncate text-xs opacity-50">
                        {{ rowSummary(item) }}
                      </p>
                    </div>
                    <label
                      class="app-switch shrink-0"
                      @pointerdown.stop
                      @click.stop.prevent="toggleEnabled(item)"
                    >
                      <input
                        type="checkbox"
                        tabindex="-1"
                        :checked="item.enabled"
                        :aria-label="item.enabled ? t('filter.disable') : t('filter.enable')"
                      />
                      <span class="app-switch-track"></span>
                    </label>
                    <IconChevronRight class="h-4 w-4 shrink-0 opacity-35" />
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
    stacked
    :show="show && addOpen"
    :title="t('filter.add')"
    :actions="addActions"
    @close="addOpen = false"
    @select="handleAdd"
  />
  <FilterEditorSheet
    :show="show && !!editingItem"
    :is-dark="isDark"
    :item="editingItem"
    :avail="avail"
    :title="editingItem ? filterTypeName(editingItem.type) : ''"
    @close="closeEditor"
    @apply="applyFilters"
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
