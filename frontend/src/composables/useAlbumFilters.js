import { computed, nextTick, ref, watch } from "vue";
import { t } from "../i18n/index.js";
import {
  FILTER_DEF_MAP,
  FILTER_REGISTRY,
  compileAlbumFilterClauses,
  filterStorageKey,
  loadFilterItemsFromStorage,
} from "../utils/filterDefs.js";

export const LOGIC_CYCLE = ["and", "or", "and_not"];
export const LOGIC_LABELS = { and: "且", or: "或", and_not: "且非" };
export const LOGIC_TITLE = {
  and: "AND with previous",
  or: "OR with previous",
  and_not: "AND NOT previous",
};

export const GROUP_LABEL_KEYS = {
  all: "collections.all",
  date: "collections.byDate",
  camera: "collections.byCamera",
  lens: "collections.byLens",
  tag: "collections.byTag",
  rating: "collections.byRating",
  color: "collections.byColor",
  format: "collections.byFormat",
};

export const COLOR_SWATCH = {
  red: "bg-red-500",
  yellow: "bg-yellow-400",
  green: "bg-emerald-500",
  blue: "bg-blue-500",
  purple: "bg-purple-500",
};

let _nextId = 1;
function uid() {
  return `fi${_nextId++}`;
}

export function filterTypeName(type) {
  return FILTER_DEF_MAP[type] ? t(FILTER_DEF_MAP[type].labelKey) : type;
}

export function filterValueSummary(item, avail = {}) {
  const value = item?.value || {};
  switch (item?.type) {
    case "date_range": {
      if (!value.from && !value.to) return "";
      return `${value.from || "…"} → ${value.to || "…"}`;
    }
    case "rating": {
      const ratings = Array.isArray(value.ratings) ? value.ratings : [];
      if (!ratings.length) return "";
      return ratings.map((star) => (star === 0 ? "0" : `${star}★`)).join("  ");
    }
    case "color_label":
      return Array.isArray(value.colorLabels) ? value.colorLabels.join(", ") : "";
    case "format":
      return value.format || "";
    case "tags": {
      const tags = String(value.tags || "")
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
      return tags.map((tag) => `#${tag}`).join("  ");
    }
    case "camera":
      return value.camera || "";
    case "lens":
      return value.lens || "";
    case "text":
      return String(value.q || "").trim();
    case "person": {
      const ids = Array.isArray(value.personIds) ? value.personIds : [];
      if (!ids.length) return "";
      const people = avail.availablePeople || [];
      const names = ids
        .map((id) => people.find((person) => person.id === id))
        .map((person) => String(person?.name || "").trim())
        .filter(Boolean);
      if (names.length) return names.join(", ");
      return String(ids.length);
    }
    default:
      return "";
  }
}

export function groupLabel(id) {
  return GROUP_LABEL_KEYS[id] ? t(GROUP_LABEL_KEYS[id]) : id;
}

export function colorKey(child) {
  return child.id.includes(":") ? child.id.split(":")[1] : child.label?.toLowerCase();
}

function defaultValue(type) {
  return FILTER_DEF_MAP[type] ? FILTER_DEF_MAP[type].defaultValue() : {};
}

/**
 * Shared album filter + collection state used by the desktop sidebar
 * and the mobile filter sheet.
 */
export function useAlbumFilters(props, emit) {
  const selectedGroupId = ref("all");
  const collectionFilter = ref({});
  const treeExpanded = ref([]);
  const addFilterOpen = ref(false);
  const hydrating = ref(false);

  const avail = computed(() => ({
    availableFormats: props.availableFormats,
    availableCameras: props.availableCameras,
    availableLenses: props.availableLenses,
    availableTags: props.availableTags,
    availablePeople: props.availablePeople,
    dateHistogram: props.dateHistogram,
    currentAlbum: props.currentAlbum,
    currentUser: props.currentUser,
  }));

  const currentGroup = computed(
    () => props.collections.find((n) => n.id === selectedGroupId.value) || null,
  );

  const storageKey = computed(() => filterStorageKey(props.currentUser, props.currentAlbum));

  function makeDefaultFilters() {
    return loadFilterItemsFromStorage(props.currentUser, props.currentAlbum).map((item) => ({
      ...item,
      id: uid(),
    }));
  }

  const filterItems = ref(makeDefaultFilters());

  function compileFilters() {
    return {
      clauses: compileAlbumFilterClauses(filterItems.value, collectionFilter.value),
    };
  }

  function compileAndUpdate() {
    const compiled = compileFilters();
    props.filters.clauses = compiled.clauses;
  }

  function applyFilters() {
    if (hydrating.value) return;
    compileAndUpdate();
    emit("apply");
  }

  async function loadFilters() {
    hydrating.value = true;
    try {
      if (!props.currentAlbum) {
        filterItems.value = makeDefaultFilters();
        compileAndUpdate();
        return;
      }
      filterItems.value = loadFilterItemsFromStorage(props.currentUser, props.currentAlbum).map((item) => ({
        ...item,
        id: uid(),
      }));
      compileAndUpdate();
    } finally {
      await nextTick();
      hydrating.value = false;
    }
  }

  function saveFilters() {
    if (props.persistFilters === false || hydrating.value || !props.currentAlbum) return;
    try {
      localStorage.setItem(storageKey.value, JSON.stringify(filterItems.value));
    } catch {
      /* ignore */
    }
  }

  watch(() => props.currentAlbum, loadFilters, { immediate: true });
  watch(filterItems, saveFilters, { deep: true });
  watch(selectedGroupId, () => {
    treeExpanded.value = [];
  });
  watch(
    () => props.currentAlbum,
    () => {
      selectedGroupId.value = "all";
      collectionFilter.value = {};
      treeExpanded.value = [];
    },
  );

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

  function isTreeExpanded(id) {
    return treeExpanded.value.includes(id);
  }

  function toggleTree(id) {
    const idx = treeExpanded.value.indexOf(id);
    if (idx >= 0) treeExpanded.value.splice(idx, 1);
    else treeExpanded.value.push(id);
  }

  function selectCollectionNode(node) {
    if (node.children && node.children.length > 0) {
      toggleTree(node.id);
      if (node.filter == null) return;
    }
    collectionFilter.value = node.filter || {};
    emit("select-collection", node);
    applyFilters();
  }

  function addFilter(type) {
    filterItems.value.push({
      id: uid(),
      type,
      logic: "and",
      enabled: true,
      value: defaultValue(type),
    });
    addFilterOpen.value = false;
    applyFilters();
    return filterItems.value[filterItems.value.length - 1];
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

  function setFilterEnabled(item, enabled) {
    if (item.enabled === enabled) return;
    item.enabled = enabled;
    applyFilters();
  }

  function cycleLogic(item) {
    const idx = LOGIC_CYCLE.indexOf(item.logic);
    item.logic = LOGIC_CYCLE[(idx + 1) % LOGIC_CYCLE.length];
    applyFilters();
  }

  function onFilterReorder() {
    applyFilters();
  }

  function onFilterDragStart(event) {
    const dt = event?.originalEvent?.dataTransfer;
    if (!dt) return;
    const blank = document.createElement("canvas");
    blank.width = 1;
    blank.height = 1;
    dt.setDragImage(blank, 0, 0);
  }

  function logicClass(logic, isDark) {
    if (logic === "and") {
      return isDark ? "bg-emerald-900/70 text-emerald-300" : "bg-emerald-100 text-emerald-700";
    }
    if (logic === "or") {
      return isDark ? "bg-sky-900/70 text-sky-300" : "bg-sky-100 text-sky-700";
    }
    if (logic === "and_not") {
      return isDark ? "bg-rose-900/70 text-rose-300" : "bg-rose-100 text-rose-700";
    }
    return "";
  }

  return {
    FILTER_REGISTRY,
    filterItems,
    hydrating,
    avail,
    addFilterOpen,
    selectedGroupId,
    currentGroup,
    treeExpanded,
    collectionFilter,
    addFilter,
    deleteFilter,
    resetFilter,
    resetAllFilters,
    toggleFilterEnabled,
    setFilterEnabled,
    cycleLogic,
    onFilterReorder,
    onFilterDragStart,
    applyFilters,
    isTreeExpanded,
    toggleTree,
    selectCollectionNode,
    logicClass,
  };
}
