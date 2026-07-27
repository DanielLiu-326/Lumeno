<script setup>
import { ref, computed, watch, onMounted } from "vue";
import { IconClose, IconRefresh, IconPlus, IconImage, IconSort } from "../utils/icons";
import { t } from "../i18n/index.js";
import { FILTER_REGISTRY, FILTER_DEF_MAP, buildFilterClauses } from "../utils/filterDefs.js";

// ────── Props ──────────────────────────────────────────────────────────────
const props = defineProps({
  isDark: { type: Boolean, default: false },
  isMobile: { type: Boolean, default: false },
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

const SORT_FIELDS = [
  { key: "date", label: () => t("albums.sortDate") },
  { key: "name", label: () => t("albums.sortName") },
  { key: "size", label: () => t("albums.sortSize") },
  { key: "rating", label: () => t("albums.sortRating") },
  { key: "camera", label: () => t("albums.sortCamera") },
  { key: "lens", label: () => t("albums.sortLens") },
];

function addSortLevel() {
  const used = props.sort.map((s) => s.field);
  const next = SORT_FIELDS.find((f) => !used.includes(f.key));
  if (!next) return;
  emit("update-sort", [...props.sort, { field: next.key, dir: "desc" }]);
}

function removeSortLevel(index) {
  const next = [...props.sort];
  next.splice(index, 1);
  emit("update-sort", next);
}

function setSortField(index, field) {
  const next = props.sort.map((level, i) => (i === index ? { ...level, field } : level));
  emit("update-sort", next);
}

function toggleSortDir(index) {
  const next = props.sort.map((level, i) =>
    i === index ? { ...level, dir: level.dir === "asc" ? "desc" : "asc" } : level,
  );
  emit("update-sort", next);
}

function clearSort() {
  emit("update-sort", []);
}

// ────── Sidebar resizing (PC only) ─────────────────────────────────────────
const isResizing = ref(false);
const sidebarWidth = ref(240); // 240px is same as w-60

let startWidth = 0;
let startX = 0;

function startResize(e) {
  if (props.isMobile) return;
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
  sidebarWidth.value = Math.max(180, Math.min(600, startWidth + deltaX));
}

function stopResize() {
  isResizing.value = false;
  document.removeEventListener("mousemove", resize);
  document.removeEventListener("mouseup", stopResize);
  document.body.style.userSelect = "";
}

// ═══════════════════════════════════════════════════════
// SECTION 1: Collections
// ═══════════════════════════════════════════════════════

const selectedGroupId = ref("all");
const collectionFilter = ref({});
const treeExpanded = ref([]); // array of expanded node IDs

const GROUP_LABEL_KEYS = {
  all: "collections.all",
  date: "collections.byDate",
  camera: "collections.byCamera",
  lens: "collections.byLens",
  tag: "collections.byTag",
  rating: "collections.byRating",
  color: "collections.byColor",
  format: "collections.byFormat",
};

function groupLabel(id) {
  return GROUP_LABEL_KEYS[id] ? t(GROUP_LABEL_KEYS[id]) : id;
}

const currentGroup = computed(() =>
  props.collections.find((n) => n.id === selectedGroupId.value) || null,
);

// Reset tree expansion when group changes
watch(selectedGroupId, () => { treeExpanded.value = []; });

// Reset when album changes
watch(() => props.currentAlbum, () => {
  selectedGroupId.value = "all";
  collectionFilter.value = {};
  treeExpanded.value = [];
});

function isTreeExpanded(id) {
  return treeExpanded.value.includes(id);
}

function toggleTree(id) {
  const idx = treeExpanded.value.indexOf(id);
  if (idx >= 0) treeExpanded.value.splice(idx, 1);
  else treeExpanded.value.push(id);
}

function selectCollectionNode(node) {
  // Toggle expand for nodes that have children
  if (node.children && node.children.length > 0) {
    toggleTree(node.id);
    if (node.filter == null) return; // pure group container, no filter
  }
  // Apply this node's filter (empty = all photos)
  collectionFilter.value = node.filter || {};
  emit("select-collection", node);
  applyFilters();
}

// Color key from child id "color:red" → "red"
function colorKey(child) {
  return child.id.includes(":") ? child.id.split(":")[1] : child.label?.toLowerCase();
}

const COLOR_SWATCH = {
  red: "bg-red-500",
  yellow: "bg-yellow-400",
  green: "bg-emerald-500",
  blue: "bg-blue-500",
  purple: "bg-purple-500",
};

// ═══════════════════════════════════════════════════════
// SECTION 2: Filter items
// ═══════════════════════════════════════════════════════

// Logic options cycle: AND → OR → AND NOT → AND
const LOGIC_CYCLE  = ["and", "or", "and_not"];
const LOGIC_LABELS = { and: "且", or: "或", and_not: "且非" };
const LOGIC_TITLE  = { and: "AND", or: "OR", and_not: "AND NOT" };

let _nextId = 1;
function uid() { return `fi${_nextId++}`; }

function defaultValue(type) {
  return FILTER_DEF_MAP[type] ? FILTER_DEF_MAP[type].defaultValue() : {};
}

function makeDefaultFilters() {
  return FILTER_REGISTRY
    .filter((def) => def.defaultEnabled)
    .map((def) => ({ id: uid(), type: def.key, logic: "and", enabled: true, value: def.defaultValue() }));
}

// Bundle all per-filter availability data into one object passed to each component
const avail = computed(() => ({
  availableFormats: props.availableFormats,
  availableCameras: props.availableCameras,
  availableLenses:  props.availableLenses,
  availableTags:    props.availableTags,
  availablePeople:  props.availablePeople,
  dateHistogram:    props.dateHistogram,
  currentAlbum:     props.currentAlbum,
  currentUser:      props.currentUser,
}));

const filterItems = ref(makeDefaultFilters());

// ── LocalStorage persistence ─────────────────────────────────────────
const storageKey = computed(
  () => `album_filters_v1_${props.currentUser}_${props.currentAlbum}`,
);

function loadFilters() {
  if (!props.currentAlbum) {
    filterItems.value = makeDefaultFilters();
    compileAndUpdate();
    return;
  }
  try {
    const saved = localStorage.getItem(storageKey.value);
    if (saved) {
      const parsed = JSON.parse(saved);
      if (Array.isArray(parsed) && parsed.length > 0) {
        filterItems.value = parsed.map((item) => ({ ...item, id: uid() }));
        compileAndUpdate();
        return;
      }
    }
  } catch { /* ignore */ }
  filterItems.value = makeDefaultFilters();
  compileAndUpdate();
}

function saveFilters() {
  if (!props.currentAlbum) return;
  try {
    localStorage.setItem(storageKey.value, JSON.stringify(filterItems.value));
  } catch { /* ignore */ }
}

watch(() => props.currentAlbum, loadFilters, { immediate: true });
watch(filterItems, saveFilters, { deep: true });

// Keep person filter item in sync when AI tools apply a person filter externally.
watch(
  () => props.filters?.clauses,
  (clauses) => {
    const personClause = (clauses || []).find((c) => c.type === "person" && c.enabled);
    const ids = personClause?.value?.personIds;
    if (!Array.isArray(ids) || ids.length === 0) return;
    const existing = filterItems.value.find((item) => item.type === "person");
    if (existing) {
      const current = existing.value?.personIds || [];
      if (current.length === ids.length && current.every((id, i) => id === ids[i])) return;
      existing.value = { personIds: [...ids] };
      existing.enabled = true;
      existing.logic = personClause.logic || "and";
      return;
    }
    filterItems.value.push({
      id: uid(),
      type: "person",
      logic: personClause.logic || "and",
      enabled: true,
      value: { personIds: [...ids] },
    });
  },
  { deep: true },
);

// ── Filter item operations ───────────────────────────────────────────

const addFilterOpen = ref(false);

function addFilter(type) {
  filterItems.value.push({ id: uid(), type, logic: "and", enabled: true, value: defaultValue(type) });
  addFilterOpen.value = false;
  applyFilters();
}

function deleteFilter(id) {
  filterItems.value = filterItems.value.filter((f) => f.id !== id);
  applyFilters();
}

function resetFilter(item) {
  item.value = defaultValue(item.type);
  applyFilters();
}

function resetAllFilters() {
  filterItems.value = makeDefaultFilters();
  collectionFilter.value = {};
  selectedGroupId.value = "all";
  treeExpanded.value = [];
  const allNode = props.collections.find((n) => n.id === "all");
  emit("select-collection", allNode || { id: "all", filter: null });
  applyFilters();
}

function toggleFilterEnabled(item) {
  item.enabled = !item.enabled;
  applyFilters();
}

function cycleLogic(item) {
  const idx = LOGIC_CYCLE.indexOf(item.logic);
  item.logic = LOGIC_CYCLE[(idx + 1) % LOGIC_CYCLE.length];
  applyFilters();
}

function filterTypeName(type) {
  return FILTER_DEF_MAP[type] ? t(FILTER_DEF_MAP[type].labelKey) : type;
}

function logicClass(logic) {
  if (logic === "and") {
    return props.isDark
      ? "bg-emerald-900/70 text-emerald-300"
      : "bg-emerald-100 text-emerald-700";
  }
  if (logic === "or") {
    return props.isDark ? "bg-sky-900/70 text-sky-300" : "bg-sky-100 text-sky-700";
  }
  if (logic === "and_not") {
    return props.isDark ? "bg-rose-900/70 text-rose-300" : "bg-rose-100 text-rose-700";
  }
  return "";
}

// ── Compile + apply ──────────────────────────────────────────────────

function compileFilters() {
  return {
    clauses: buildFilterClauses(filterItems.value, collectionFilter.value),
  };
}

// Update albumFilters without triggering a refresh (used during album-change
// so selectAlbum's deferred refresh reads the correct compiled values).
function compileAndUpdate() {
  const compiled = compileFilters();
  props.filters.clauses = compiled.clauses;
}

function applyFilters() {
  compileAndUpdate();
  emit("apply");
}

// When FilterSidebar mounts (filter panel opens), apply saved filters and
// trigger a photo refresh so the list matches the restored filter state.
onMounted(() => {
  applyFilters();
});
</script>

<template>
  <aside
    class="relative app-scrollbar flex shrink-0 flex-col overflow-y-auto border-r"
    :class="[
      isDark ? 'border-stone-700/60 bg-stone-900' : 'border-stone-200/60 bg-stone-50',
      isMobile
        ? 'fixed inset-y-0 left-0 z-[91] w-[min(18rem,85vw)] shadow-2xl'
        : '',
    ]"
    :style="isMobile ? {} : { width: `${sidebarWidth}px` }"
  >
    <!-- Drag handle for resizing (PC only) -->
    <div
      v-if="!isMobile"
      class="absolute top-0 right-0 bottom-0 w-1.5 cursor-col-resize hover:bg-amber-500/40 transition-colors z-[100]"
      @mousedown="startResize"
    ></div>

    <!-- ── Mobile header ── -->
    <div
      v-if="isMobile"
      class="flex shrink-0 items-center justify-between border-b px-3 py-2.5"
      :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'"
    >
      <p class="text-xs font-bold" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
        {{ t("albums.filters") }}
      </p>
      <button
        class="rounded-full p-1.5 transition"
        :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
        @click="emit('close')"
      >
        <IconClose class="h-3.5 w-3.5" />
      </button>
    </div>

    <!-- ═══════════════════════════════════════
         SECTION 1: COLLECTIONS
         ═══════════════════════════════════════ -->
    <div
      v-if="collections.length > 0"
      class="shrink-0 border-b px-3 py-2.5"
      :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'"
    >
      <p
        class="mb-2 text-[10px] font-bold uppercase tracking-wider"
        :class="isDark ? 'text-stone-500' : 'text-stone-400'"
      >
        {{ t("collections.title") }}
      </p>

      <!-- Group selector combobox -->
      <select
        v-model="selectedGroupId"
        class="mb-2 w-full rounded-lg border px-2 py-1.5 text-xs"
        :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
      >
        <option v-for="node in collections" :key="node.id" :value="node.id">
          {{ groupLabel(node.id) }}
        </option>
      </select>

      <!-- Tree for selected group -->
      <div v-if="currentGroup" class="app-scrollbar flex max-h-52 flex-col gap-0.5 overflow-y-auto">

        <!-- "all" group: single All Photos button -->
        <template v-if="currentGroup.id === 'all'">
          <button
            class="flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-xs font-medium transition"
            :class="selectedCollectionId === 'all'
              ? (isDark ? 'bg-amber-500/25 text-amber-200' : 'bg-amber-100 text-amber-900')
              : (isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-600 hover:bg-stone-200')"
            @click="selectCollectionNode(currentGroup)"
          >
            <IconImage class="h-3 w-3 shrink-0" />
            <span class="min-w-0 flex-1 truncate">{{ t("collections.all") }}</span>
            <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
              {{ currentGroup.count }}
            </span>
          </button>
        </template>

        <!-- Group with children (date, camera, lens, etc.) -->
        <template v-else>
          <template v-for="child in (currentGroup.children || [])" :key="child.id">

            <!-- Child with sub-children (e.g. year → months) -->
            <template v-if="child.children && child.children.length > 0">
              <button
                class="flex w-full items-center gap-1 rounded-lg px-2 py-1 text-left text-xs transition"
                :class="selectedCollectionId === child.id
                  ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                  : (isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-600 hover:bg-stone-200')"
                @click="selectCollectionNode(child)"
              >
                <svg
                  class="h-2.5 w-2.5 shrink-0 transition-transform"
                  :class="isTreeExpanded(child.id) ? 'rotate-90' : ''"
                  viewBox="0 0 20 20" fill="currentColor"
                >
                  <path fill-rule="evenodd" clip-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z"/>
                </svg>
                <span class="min-w-0 flex-1 truncate">{{ child.label }}</span>
                <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  {{ child.count }}
                </span>
              </button>
              <!-- Grandchildren (months) -->
              <template v-if="isTreeExpanded(child.id)">
                <button
                  v-for="gc in child.children"
                  :key="gc.id"
                  class="flex w-full items-center gap-1 rounded-lg pl-6 pr-2 py-0.5 text-left text-xs transition"
                  :class="selectedCollectionId === gc.id
                    ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                    : (isDark ? 'text-stone-400 hover:bg-stone-800' : 'text-stone-500 hover:bg-stone-200')"
                  @click="selectCollectionNode(gc)"
                >
                  <span class="min-w-0 flex-1 truncate">{{ gc.label }}</span>
                  <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-600' : 'text-stone-400'">
                    {{ gc.count }}
                  </span>
                </button>
              </template>
            </template>

            <!-- Leaf child -->
            <button
              v-else
              class="flex w-full items-center gap-1.5 rounded-lg px-2 py-1 text-left text-xs transition"
              :class="selectedCollectionId === child.id
                ? (isDark ? 'bg-amber-500/25 text-amber-200 font-medium' : 'bg-amber-100 text-amber-900 font-medium')
                : (isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-600 hover:bg-stone-200')"
              @click="selectCollectionNode(child)"
            >
              <!-- Color swatch for color group -->
              <span
                v-if="currentGroup.id === 'color' && COLOR_SWATCH[colorKey(child)]"
                class="h-2.5 w-2.5 shrink-0 rounded-full"
                :class="COLOR_SWATCH[colorKey(child)]"
              ></span>
              <span class="min-w-0 flex-1 truncate">{{ child.label }}</span>
              <span class="shrink-0 tabular-nums text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                {{ child.count }}
              </span>
            </button>

          </template>
        </template>
      </div>
    </div>

    <!-- ═══════════════════════════════════════
         SECTION: SORT
         ═══════════════════════════════════════ -->
    <div class="shrink-0 border-b px-3 py-2.5" :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'">
      <div class="mb-2 flex items-center gap-1">
        <IconSort class="h-3 w-3" :class="isDark ? 'text-stone-500' : 'text-stone-400'" />
        <p class="flex-1 text-[10px] font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t("albums.sort") }}
        </p>
        <button
          class="rounded px-1.5 py-0.5 text-[10px] font-medium transition"
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
            class="min-w-0 flex-1 rounded-lg border px-2 py-1.5 text-xs"
            :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
            @change="setSortField(idx, $event.target.value)"
          >
            <option
              v-for="f in SORT_FIELDS"
              :key="f.key"
              :value="f.key"
              :disabled="sort.some((s, i) => i !== idx && s.field === f.key)"
            >{{ f.label() }}</option>
          </select>
          <button
            class="rounded-lg border px-2 py-1.5 text-xs font-medium transition"
            :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-200 hover:bg-stone-700' : 'border-stone-300 bg-white text-stone-700 hover:bg-stone-100'"
            @click="toggleSortDir(idx)"
          >
            {{ level.dir === "asc" ? "↑" : "↓" }}
          </button>
          <button
            class="rounded-lg p-1.5 text-xs transition"
            :class="isDark ? 'text-rose-400 hover:bg-rose-900/30' : 'text-rose-500 hover:bg-rose-100'"
            @click="removeSortLevel(idx)"
          >
            <IconClose class="h-3 w-3" />
          </button>
        </div>
        <button
          v-if="sort.length < SORT_FIELDS.length"
          class="inline-flex items-center gap-1 rounded-lg px-2 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
          @click="addSortLevel"
        >
          <IconPlus class="h-3 w-3" />{{ t("albums.addSortLevel") }}
        </button>
      </div>
    </div>

    <!-- ═══════════════════════════════════════
         SECTION 2: FILTERS
         ═══════════════════════════════════════ -->
    <div class="flex min-h-0 flex-1 flex-col overflow-y-auto px-3 py-2.5">

      <!-- Filters header row -->
      <div class="mb-2 flex shrink-0 items-center gap-1">
        <p
          class="flex-1 text-[10px] font-bold uppercase tracking-wider"
          :class="isDark ? 'text-stone-500' : 'text-stone-400'"
        >
          {{ t("albums.filters") }}
        </p>
        <!-- Reset all -->
        <button
          class="rounded p-1 transition"
          :class="isDark ? 'text-stone-500 hover:text-stone-300 hover:bg-stone-800' : 'text-stone-400 hover:text-stone-600 hover:bg-stone-200'"
          :title="t('filter.resetAll')"
          @click="resetAllFilters"
        >
          <IconRefresh class="h-3 w-3" />
        </button>
        <!-- Add filter -->
        <div class="relative">
          <button
            class="flex items-center rounded p-1 transition"
            :class="isDark ? 'text-stone-400 hover:text-stone-200 hover:bg-stone-800' : 'text-stone-500 hover:text-stone-700 hover:bg-stone-200'"
            :title="t('filter.add')"
            @click="addFilterOpen = !addFilterOpen"
          >
            <IconPlus class="h-3 w-3" />
          </button>
          <!-- Dropdown backdrop -->
          <div v-if="addFilterOpen" class="fixed inset-0 z-[20]" @click="addFilterOpen = false"></div>
          <!-- Dropdown menu -->
          <div
            v-if="addFilterOpen"
            class="absolute right-0 top-full z-[21] mt-1 w-36 rounded-xl border py-1 shadow-lg"
            :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'"
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
        </div>
      </div>

      <!-- Filter item cards -->
      <div class="flex flex-col gap-2">
        <div
          v-for="item in filterItems"
          :key="item.id"
          class="rounded-xl border transition"
          :class="[
            isDark ? 'border-stone-700' : 'border-stone-200',
            !item.enabled ? 'opacity-40' : '',
          ]"
        >
          <!-- Card header -->
          <div class="flex items-center gap-1 px-2 py-1.5">
            <!-- Logic badge: click to cycle -->
            <button
              class="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-bold leading-none"
              :class="logicClass(item.logic)"
              :title="LOGIC_TITLE[item.logic]"
              @click="cycleLogic(item)"
            >
              {{ LOGIC_LABELS[item.logic] }}
            </button>
            <!-- Filter type label -->
            <span
              class="min-w-0 flex-1 truncate text-xs font-medium"
              :class="isDark ? 'text-stone-300' : 'text-stone-700'"
            >
              {{ filterTypeName(item.type) }}
            </span>
            <!-- Reset -->
            <button
              class="shrink-0 rounded p-0.5 transition"
              :class="isDark ? 'text-stone-500 hover:text-stone-300' : 'text-stone-400 hover:text-stone-600'"
              :title="t('filter.reset')"
              @click="resetFilter(item)"
            >
              <IconRefresh class="h-3 w-3" />
            </button>
            <!-- Toggle enable/disable -->
            <button
              class="shrink-0 rounded p-0.5 transition"
              :class="item.enabled
                ? (isDark ? 'text-amber-400 hover:text-amber-300' : 'text-amber-500 hover:text-amber-700')
                : (isDark ? 'text-stone-600 hover:text-stone-400' : 'text-stone-300 hover:text-stone-500')"
              :title="item.enabled ? t('filter.disable') : t('filter.enable')"
              @click="toggleFilterEnabled(item)"
            >
              <!-- Circle icon (power-like) -->
              <svg class="h-3 w-3" viewBox="0 0 20 20" fill="currentColor">
                <path fill-rule="evenodd" clip-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm0-2a6 6 0 100-12 6 6 0 000 12z"/>
              </svg>
            </button>
            <!-- Delete -->
            <button
              class="shrink-0 rounded p-0.5 transition"
              :class="isDark ? 'text-stone-500 hover:text-rose-400' : 'text-stone-400 hover:text-rose-500'"
              :title="t('filter.delete')"
              @click="deleteFilter(item.id)"
            >
              <IconClose class="h-3 w-3" />
            </button>
          </div>

          <!-- Card content (only shown when enabled) -->
          <div
            v-if="item.enabled && FILTER_DEF_MAP[item.type]?.component"
            class="min-w-0 overflow-hidden border-t px-2 pb-2 pt-2"
            :class="isDark ? 'border-stone-700/50' : 'border-stone-200'"
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
      </div>
    </div>
  </aside>
</template>
