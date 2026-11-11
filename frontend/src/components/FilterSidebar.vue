<script setup>
import { onBeforeUnmount, ref, watch } from "vue";
import { VueDraggable } from "vue-draggable-plus";
import { IconClose, IconRefresh, IconPlus, IconSort, IconGripVertical } from "../utils/icons";
import { t } from "../i18n/index.js";
import { FILTER_DEF_MAP, SORT_FIELDS, withAddedSortLevel, withRemovedSortLevel, withSortField, withToggledSortDir } from "../utils/filterDefs.js";
import {
  LOGIC_LABELS,
  LOGIC_TITLE,
  filterTypeName,
  useAlbumFilters,
} from "../composables/useAlbumFilters.js";
import FilterCollections from "./FilterCollections.vue";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  collections: { type: Array, default: () => [] },
  selectedCollectionId: { type: String, default: "all" },
  availableTags: { type: Array, default: () => [] },
  availableCameras: { type: Array, default: () => [] },
  availableLenses: { type: Array, default: () => [] },
  availableFormats: { type: Array, default: () => [] },
  dateHistogram: { type: Array, default: () => [] },
  filters: { type: Object, required: true },
  sort: { type: Array, default: () => [] },
  availablePeople: { type: Array, default: () => [] },
  currentUser: { type: String, default: "" },
  currentAlbum: { type: String, default: "" },
});

const emit = defineEmits(["apply", "select-collection", "close", "update-sort"]);

const {
  FILTER_REGISTRY,
  filterItems,
  avail,
  addFilterOpen,
  selectedGroupId,
  currentGroup,
  treeExpanded,
  addFilter,
  deleteFilter,
  resetFilter,
  resetAllFilters,
  toggleFilterEnabled,
  cycleLogic,
  onFilterReorder,
  onFilterDragStart,
  applyFilters,
  selectCollectionNode,
  logicClass,
} = useAlbumFilters(props, emit);

const SIDEBAR_DEFAULT_WIDTH = 272;
const SIDEBAR_MIN_WIDTH = 220;
const SIDEBAR_MAX_WIDTH = 600;
const SIDEBAR_WIDTH_STORAGE_PREFIX = "lumeno.filterSidebarWidth";

const isResizing = ref(false);
const sidebarWidth = ref(loadSidebarWidth());

let startWidth = 0;
let startX = 0;

function clampSidebarWidth(value) {
  return Math.max(SIDEBAR_MIN_WIDTH, Math.min(SIDEBAR_MAX_WIDTH, value));
}

function sidebarWidthStorageKey() {
  return `${SIDEBAR_WIDTH_STORAGE_PREFIX}.${props.currentUser || "guest"}`;
}

function loadSidebarWidth() {
  try {
    const raw = localStorage.getItem(sidebarWidthStorageKey());
    const value = Number(raw);
    return Number.isFinite(value) ? clampSidebarWidth(value) : SIDEBAR_DEFAULT_WIDTH;
  } catch {
    return SIDEBAR_DEFAULT_WIDTH;
  }
}

function saveSidebarWidth() {
  try {
    localStorage.setItem(sidebarWidthStorageKey(), String(sidebarWidth.value));
  } catch {
    /* ignore quota / private mode */
  }
}

function resetSidebarWidth() {
  sidebarWidth.value = SIDEBAR_DEFAULT_WIDTH;
  saveSidebarWidth();
}

function startResize(e) {
  e.preventDefault();
  isResizing.value = true;
  startWidth = sidebarWidth.value;
  startX = e.clientX;
  document.addEventListener("mousemove", resize);
  document.addEventListener("mouseup", stopResize);
  document.body.style.userSelect = "none";
}

function resize(e) {
  if (!isResizing.value) return;
  const deltaX = e.clientX - startX;
  sidebarWidth.value = clampSidebarWidth(startWidth + deltaX);
}

function stopResize() {
  if (!isResizing.value) return;
  isResizing.value = false;
  document.removeEventListener("mousemove", resize);
  document.removeEventListener("mouseup", stopResize);
  document.body.style.userSelect = "";
  saveSidebarWidth();
}

watch(
  () => props.currentUser,
  () => {
    sidebarWidth.value = loadSidebarWidth();
  },
);

onBeforeUnmount(() => {
  stopResize();
  document.removeEventListener("mousemove", resize);
  document.removeEventListener("mouseup", stopResize);
});

function addSortLevel() {
  emit("update-sort", withAddedSortLevel(props.sort));
}

function removeSortLevel(index) {
  emit("update-sort", withRemovedSortLevel(props.sort, index));
}

function setSortField(index, field) {
  emit("update-sort", withSortField(props.sort, index, field));
}

function toggleSortDir(index) {
  emit("update-sort", withToggledSortDir(props.sort, index));
}

function clearSort() {
  emit("update-sort", []);
}

function filterCardBaseClass() {
  return "app-surface-soft app-card";
}
</script>

<template>
  <aside
    class="app-scrollbar relative flex h-full shrink-0 flex-col overflow-y-auto app-split-r app-frost-sidebar"
    :style="{ width: `${sidebarWidth}px` }"
  >
    <div
      class="absolute top-0 right-0 bottom-0 w-1.5 cursor-col-resize hover:bg-amber-500/40 transition-colors z-[100]"
      :title="t('albums.sidebarResizeHint')"
      @mousedown="startResize"
      @dblclick="resetSidebarWidth"
    ></div>

    <div
      v-if="collections.length > 0"
      class="shrink-0 app-split-b px-3 py-2.5"
    >
      <FilterCollections
        :is-dark="isDark"
        :collections="collections"
        :selected-collection-id="selectedCollectionId"
        :selected-group-id="selectedGroupId"
        :current-group="currentGroup"
        :tree-expanded="treeExpanded"
        @update:selected-group-id="selectedGroupId = $event"
        @select-node="selectCollectionNode"
      />
    </div>

    <div class="shrink-0 app-split-b px-3 py-2.5">
      <div class="mb-2 flex items-center gap-1.5">
        <IconSort class="h-3.5 w-3.5" :class="isDark ? 'text-stone-500' : 'text-stone-400'" />
        <p class="flex-1 text-[11px] font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t("albums.sort") }}
        </p>
        <button
          class="rounded-md px-1.5 py-0.5 text-[11px] font-medium transition"
          :class="isDark ? 'text-stone-500 hover:bg-stone-800 hover:text-stone-300' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-600'"
          @click="clearSort"
        >
          {{ t("albums.clear") }}
        </button>
      </div>
      <div class="flex flex-col gap-1.5">
        <div v-for="(level, idx) in sort" :key="idx" class="flex items-center gap-1.5">
          <select
            :value="level.field"
            class="min-w-0 flex-1 rounded-lg border px-2 py-1.5 text-xs app-control"
            :class="isDark ? 'text-stone-100' : 'text-stone-900'"
            @change="setSortField(idx, $event.target.value)"
          >
            <option
              v-for="f in SORT_FIELDS"
              :key="f.key"
              :value="f.key"
              :disabled="sort.some((s, i) => i !== idx && s.field === f.key)"
            >{{ t(f.labelKey) }}</option>
          </select>
          <button
            class="rounded-lg border px-2 py-1.5 text-xs font-medium transition app-control"
            :class="isDark ? 'text-stone-200 hover:bg-white/10' : 'text-stone-700 hover:bg-black/5'"
            @click="toggleSortDir(idx)"
          >
            {{ level.dir === "asc" ? "↑" : "↓" }}
          </button>
          <button
            class="rounded-lg p-1.5 text-xs transition"
            :class="isDark ? 'text-rose-400 hover:bg-rose-900/30' : 'text-rose-500 hover:bg-rose-100'"
            @click="removeSortLevel(idx)"
          >
            <IconClose class="h-3.5 w-3.5" />
          </button>
        </div>
        <button
          v-if="sort.length < SORT_FIELDS.length"
          class="inline-flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs font-medium transition app-surface-soft"
          :class="isDark ? 'text-stone-200 hover:bg-white/10' : 'text-stone-700 hover:bg-black/5'"
          @click="addSortLevel"
        >
          <IconPlus class="h-3.5 w-3.5" />{{ t("albums.addSortLevel") }}
        </button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1 flex-col overflow-y-auto px-3 py-2.5">
      <div class="mb-2 flex shrink-0 items-center gap-1">
        <p
          class="flex-1 text-[11px] font-bold uppercase tracking-wider"
          :class="isDark ? 'text-stone-500' : 'text-stone-400'"
        >
          {{ t("albums.filters") }}
        </p>
        <button
          class="rounded-md p-1.5 transition"
          :class="isDark ? 'text-stone-500 hover:text-stone-300 hover:bg-stone-800' : 'text-stone-400 hover:text-stone-600 hover:bg-stone-200'"
          :title="t('filter.resetAll')"
          @click="resetAllFilters"
        >
          <IconRefresh class="h-3.5 w-3.5" />
        </button>
        <div class="relative">
          <button
            class="flex items-center rounded-md p-1.5 transition"
            :class="isDark ? 'text-stone-400 hover:text-stone-200 hover:bg-stone-800' : 'text-stone-500 hover:text-stone-700 hover:bg-stone-200'"
            :title="t('filter.add')"
            @click="addFilterOpen = !addFilterOpen"
          >
            <IconPlus class="h-3.5 w-3.5" />
          </button>
          <div v-if="addFilterOpen" class="fixed inset-0 z-[20]" @click="addFilterOpen = false"></div>
          <Transition name="app-dropdown" :duration="{ enter: 300, leave: 200 }">
          <div
            v-if="addFilterOpen"
            class="app-dropdown-surface app-frost-dense absolute right-0 top-full z-[21] mt-1 w-40 origin-top-right rounded-xl border py-1 shadow-lg"
            :class="isDark ? 'border-white/16' : 'border-stone-300/80'"
          >
            <button
              v-for="ft in FILTER_REGISTRY"
              :key="ft.key"
              class="w-full px-3 py-1.5 text-left text-xs transition"
              :class="isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-700 hover:bg-stone-100'"
              @click="addFilter(ft.key)"
            >
              {{ filterTypeName(ft.key) }}
            </button>
          </div>
          </Transition>
        </div>
      </div>

      <VueDraggable
        v-model="filterItems"
        class="flex flex-col gap-2"
        handle=".filter-drag-handle"
        :animation="220"
        direction="vertical"
        ghost-class="filter-sortable-ghost"
        drag-class="filter-sortable-drag"
        chosen-class="filter-sortable-chosen"
        @start="onFilterDragStart"
        @update="onFilterReorder"
      >
        <div
          v-for="item in filterItems"
          :key="item.id"
          data-filter-card
          class="overflow-hidden rounded-xl"
          :class="[
            filterCardBaseClass(),
            !item.enabled ? 'opacity-40' : '',
          ]"
        >
          <div class="flex items-center gap-1 py-1.5 pl-1 pr-1.5">
            <button
              type="button"
              class="filter-drag-handle -mt-0.5 shrink-0 cursor-grab touch-none rounded p-0.5 active:cursor-grabbing"
              :class="isDark ? 'text-stone-500 hover:text-stone-300' : 'text-stone-400 hover:text-stone-600'"
              :title="t('filter.dragReorder')"
            >
              <IconGripVertical class="h-3.5 w-3.5" />
            </button>
            <button
              class="shrink-0 rounded px-1.5 py-0.5 text-[11px] font-bold leading-none"
              :class="logicClass(item.logic, isDark)"
              :title="LOGIC_TITLE[item.logic]"
              @click="cycleLogic(item)"
            >
              {{ LOGIC_LABELS[item.logic] }}
            </button>
            <span
              class="min-w-0 flex-1 truncate text-xs font-medium"
              :class="isDark ? 'text-stone-300' : 'text-stone-700'"
            >
              {{ filterTypeName(item.type) }}
            </span>
            <button
              class="shrink-0 rounded p-0.5 transition"
              :class="isDark ? 'text-stone-500 hover:text-stone-300' : 'text-stone-400 hover:text-stone-600'"
              :title="t('filter.reset')"
              @click="resetFilter(item)"
            >
              <IconRefresh class="h-3.5 w-3.5" />
            </button>
            <button
              class="shrink-0 rounded p-0.5 transition"
              :class="item.enabled
                ? (isDark ? 'text-amber-400 hover:text-amber-300' : 'text-amber-500 hover:text-amber-700')
                : (isDark ? 'text-stone-600 hover:text-stone-400' : 'text-stone-300 hover:text-stone-500')"
              :title="item.enabled ? t('filter.disable') : t('filter.enable')"
              @click="toggleFilterEnabled(item)"
            >
              <svg class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="currentColor">
                <path fill-rule="evenodd" clip-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm0-2a6 6 0 100-12 6 6 0 000 12z"/>
              </svg>
            </button>
            <button
              class="shrink-0 rounded p-0.5 transition"
              :class="isDark ? 'text-stone-500 hover:text-rose-400' : 'text-stone-400 hover:text-rose-500'"
              :title="t('filter.delete')"
              @click="deleteFilter(item.id)"
            >
              <IconClose class="h-3.5 w-3.5" />
            </button>
          </div>

          <div
            v-if="item.enabled && FILTER_DEF_MAP[item.type]?.component"
            class="min-w-0 overflow-hidden app-split-b px-2 pb-2 pt-2"
          >
            <component
              :is="FILTER_DEF_MAP[item.type].component"
              :model-value="item.value"
              :is-dark="isDark"
              :avail="avail"
              @update:model-value="item.value = $event"
              @apply="applyFilters"
            />
          </div>
        </div>
      </VueDraggable>
    </div>
  </aside>
</template>

<style scoped>
:deep(.filter-sortable-ghost) {
  opacity: 0;
  border-radius: 0.75rem;
}

:deep(.filter-sortable-drag) {
  opacity: 0.55;
  border-radius: 0.75rem;
}

:deep(.filter-sortable-chosen) {
  cursor: grabbing;
  opacity: 0.55;
}
</style>
