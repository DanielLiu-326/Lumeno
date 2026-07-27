<script setup>
import { computed, nextTick, ref, watch } from "vue";
import {
  IconArrowLeft, IconCamera, IconCheck, IconClose, IconPencil, IconRefresh, IconSearch, IconTrash, IconWand,
} from "../utils/icons";
import { t } from "../i18n/index.js";
import FaceAvatar from "./FaceAvatar.vue";
import LazyImage from "./LazyImage.vue";
import VirtualGrid from "./VirtualGrid.vue";
import { personDisplayCover } from "../utils/faceCrop.js";

/** Same page size as album photo grid (`DEFAULT_PAGE_SIZE` in useAlbumApp). */
const DETAIL_PAGE_SIZE = 100;

const props = defineProps({
  isDark: { type: Boolean, default: false },
  albumName: { type: String, default: "" },
  albumOwner: { type: String, default: "" },
  busy: { type: Boolean, default: false },
  people: { type: Array, default: () => [] },
  suggestions: { type: Object, default: () => ({ clusters: [] }) },
  editable: { type: Boolean, default: true },
  initialQuery: { type: String, default: "" },
  /** (opts) => Promise<{ results, query }> — must NOT navigate away */
  onSearchText: { type: Function, default: null },
  /** () => Promise<{ groups, total }> — must NOT navigate away */
  onLoadDuplicates: { type: Function, default: null },
});

const emit = defineEmits([
  "close",
  "refresh",
  "adopt-cluster",
  "merge-cluster",
  "patch-person",
  "remove-center",
  "delete-person",
  "filter-person",
  "open-photo",
]);

const section = ref("faces");
const showSingles = ref(false);
const renameOpen = ref(false);
const renameValue = ref("");
const renamePersonId = ref("");
const deleteOpen = ref(false);
const deletePersonTarget = ref(null);
const adoptName = ref("");
const adoptClusterId = ref("");
const selectedPerson = ref(null);
/** Unnamed suggestion cluster opened for detail browsing. */
const viewingCluster = ref(null);
/** When true, person/cluster detail shows full photo thumbnails instead of face crops. */
const showOriginalThumbs = ref(false);
/** When true, person detail groups appearances by natural center clusters. */
const showNaturalClusters = ref(false);
/** Opened natural cluster within person detail (drill-down). */
const viewingPersonCenter = ref(null);

/** Segmented window of appearances/faces for the open person or cluster. */
const detailItems = ref([]);
const detailOffset = ref(0);
const detailTotal = ref(0);
const detailLoadingMore = ref(false);

const DETAIL_GRID_CLASS = "grid grid-cols-3 gap-2 sm:grid-cols-4 md:grid-cols-5";

const query = ref(props.initialQuery || "");
const minScore = ref(0);
const limit = ref(60);
const searchLoading = ref(false);
const searchResults = ref([]);
const searchError = ref("");

const dupLoading = ref(false);
const dupGroups = ref([]);
const dupError = ref("");
const dupLoaded = ref(false);

/** Scroll position of the main faces list (preserved across merge/refresh). */
const listScrollRef = ref(null);
const savedListScrollTop = ref(0);
/** After adopt/naming: show spinner until parent faces refresh completes. */
const facesSyncBusy = ref(false);

function captureListScroll() {
  if (listScrollRef.value) savedListScrollTop.value = listScrollRef.value.scrollTop;
}

function restoreListScroll() {
  nextTick(() => {
    if (listScrollRef.value) listScrollRef.value.scrollTop = savedListScrollTop.value;
  });
}

watch(
  () => props.initialQuery,
  (value) => {
    if (typeof value === "string" && value !== query.value) query.value = value;
  },
);

watch(section, (next) => {
  selectedPerson.value = null;
  viewingCluster.value = null;
  adoptClusterId.value = "";
  adoptName.value = "";
  if (next === "duplicates" && !dupLoaded.value) {
    loadDuplicates();
  }
});

// Keep person detail in sync after adopt/rename/refresh.
watch(
  () => props.people,
  (list) => {
    if (!selectedPerson.value?.id) return;
    const next = (list || []).find((p) => p.id === selectedPerson.value.id);
    if (!next) {
      showNaturalClusters.value = false;
      viewingPersonCenter.value = null;
    }
    selectedPerson.value = next || null;
  },
);

// Keep cluster detail in sync after refresh; clear if adopted away.
watch(
  () => props.suggestions?.clusters,
  (clusters) => {
    if (!viewingCluster.value?.cluster_id) return;
    const next = (clusters || []).find((c) => c.cluster_id === viewingCluster.value.cluster_id);
    viewingCluster.value = next || null;
    if (!next) {
      adoptClusterId.value = "";
      adoptName.value = "";
    }
  },
);

const namedPeople = computed(() =>
  (props.people || []).filter((p) => String(p?.name || "").trim().length > 0 && !p.hidden),
);

const allClusters = computed(() => props.suggestions?.clusters || []);
const multiClusters = computed(() => allClusters.value.filter((c) => !c.synthetic && (c.face_count || c.face_ids?.length || 0) > 1));

function clusterHasMatchSuggestions(cluster) {
  return Array.isArray(cluster?.match_suggestions) && cluster.match_suggestions.length > 0;
}

function bestMatchScore(cluster) {
  const first = cluster?.match_suggestions?.[0];
  return typeof first?.score === "number" ? first.score : 0;
}

function sortSuggestionClusters(list) {
  return [...list].sort((left, right) => {
    const leftHas = clusterHasMatchSuggestions(left);
    const rightHas = clusterHasMatchSuggestions(right);
    if (leftHas !== rightHas) return rightHas ? 1 : -1;
    if (leftHas && rightHas) {
      const byScore = bestMatchScore(right) - bestMatchScore(left);
      if (byScore !== 0) return byScore;
    }
    return String(left.cluster_id || "").localeCompare(String(right.cluster_id || ""));
  });
}

const visibleSuggestions = computed(() => {
  const singlesWithMatch = allClusters.value.filter((c) => c.synthetic && clusterHasMatchSuggestions(c));
  let list;
  if (showSingles.value) {
    list = allClusters.value;
  } else {
    const seen = new Set();
    list = [];
    for (const cluster of [...multiClusters.value, ...singlesWithMatch]) {
      if (seen.has(cluster.cluster_id)) continue;
      seen.add(cluster.cluster_id);
      list.push(cluster);
    }
  }
  return sortSuggestionClusters(list);
});

function matchPercent(score) {
  return Math.round(Math.max(0, Math.min(1, Number(score) || 0)) * 100);
}

const navItems = [
  { id: "faces", labelKey: "albums.aiFaces", icon: IconCamera },
  { id: "duplicates", labelKey: "albums.aiDuplicates", icon: IconWand },
  { id: "search", labelKey: "albums.aiTextSearch", icon: IconSearch },
];

function personLabel(person) {
  const name = String(person?.name || "").trim();
  return name || t("people.unnamed");
}

function personPreviewAppearance(person) {
  return person?.appearances?.[0] || null;
}

function personListCover(person) {
  return personDisplayCover(person, props.albumName, props.albumOwner);
}

function boxesEqual(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length < 4 || right.length < 4) {
    return false;
  }
  return left.every((value, index) => Math.abs(Number(value) - Number(right[index])) < 1e-5);
}

function isCurrentCover(appearance) {
  const person = selectedPerson.value;
  if (!person?.cover?.path || !appearance?.source) return false;
  const path = albumPhotoPath(appearance.source);
  return person.cover.path === path && boxesEqual(person.cover.face_box, detailItemBox(appearance));
}

function setPersonCover(appearance) {
  if (!selectedPerson.value?.id || !appearance?.source) return;
  emit("patch-person", selectedPerson.value.id, {
    cover: {
      source: appearance.source,
      box_xywh: detailItemBox(appearance),
    },
  });
}

function clearPersonCover() {
  if (!selectedPerson.value?.id) return;
  emit("patch-person", selectedPerson.value.id, { cover: null });
}

const personHasMultipleCenters = computed(() => {
  const person = selectedPerson.value;
  return (person?.center_count || person?.centers?.length || 0) > 1;
});

const personCentersDisplay = computed(() => {
  const person = selectedPerson.value;
  if (!person) return [];
  const meta = new Map((person.centers || []).map((center) => [center.id, center]));
  const grouped = new Map();
  for (const appearance of person.appearances || []) {
    const centerId = appearance.center_id || "unknown";
    if (!grouped.has(centerId)) grouped.set(centerId, []);
    grouped.get(centerId).push(appearance);
  }
  return [...grouped.entries()].map(([center_id, appearances]) => {
    const info = meta.get(center_id) || {};
    return {
      center_id,
      label: info.label,
      appearance_count: appearances.length,
      appearances,
    };
  });
});

watch(
  () => personCentersDisplay.value,
  (centers) => {
    if (!viewingPersonCenter.value?.center_id) return;
    const next = centers.find((c) => c.center_id === viewingPersonCenter.value.center_id);
    viewingPersonCenter.value = next || null;
  },
);

watch(showNaturalClusters, (on) => {
  if (!on) viewingPersonCenter.value = null;
});

function centerPreviewAppearances(center) {
  return (center?.appearances || []).slice(0, 16);
}

function centerClusterLabel(center) {
  const label = String(center?.label || "").trim();
  if (label) return label;
  return t("people.clusterFaceCount", { count: center?.appearance_count || 0 });
}

function openPersonCenter(center) {
  if (!center?.center_id) return;
  viewingPersonCenter.value = center;
}

function backFromPersonCenter() {
  viewingPersonCenter.value = null;
}

function removePersonCluster(center) {
  if (!props.editable || !selectedPerson.value?.id || !center?.center_id) return;
  if (!window.confirm(t("people.removeClusterConfirm"))) return;
  if (viewingPersonCenter.value?.center_id === center.center_id) {
    viewingPersonCenter.value = null;
  }
  emit("remove-center", {
    person_id: selectedPerson.value.id,
    center_id: center.center_id,
  });
}

function openPersonClusterPhoto(center, appearance) {
  const navPhotos = (center?.appearances || [])
    .map((entry) => faceItemToPhoto(entry))
    .filter(Boolean);
  const photo = faceItemToPhoto(appearance);
  if (!photo) return;
  emit("open-photo", { photo, navPhotos });
}

function albumPhotoPath(relativeSource) {
  if (!relativeSource) return "";
  if (String(relativeSource).startsWith("albums/")) return relativeSource;
  return `albums/${props.albumName}/${relativeSource}`;
}

function photoThumb(path, owner) {
  if (!path) return "";
  const ownerParam = owner ? `&owner=${encodeURIComponent(owner)}` : (props.albumOwner ? `&owner=${encodeURIComponent(props.albumOwner)}` : "");
  return `/api/thumbnail/${encodeURIComponent(path)}?size=360${ownerParam}`;
}

function basenameFromPath(path) {
  return String(path || "").split("/").pop() || path || "";
}

const faceCatalog = computed(() => props.suggestions?.face_catalog || {});

function resolveFaceItem(item) {
  if (!item) return null;
  if (item.source) return item;
  const faceId = item.face_id;
  if (!faceId) return item;
  const meta = faceCatalog.value[faceId];
  if (!meta) return item;
  return { ...item, ...meta };
}

/** Normalize AI-tool photo-like objects for AlbumPanel openDetail. */
function normalizeAiPhoto(raw) {
  if (!raw) return null;
  const source = raw.source || raw.path || "";
  const path = raw.path && String(raw.path).startsWith("albums/")
    ? raw.path
    : albumPhotoPath(source || raw.path);
  if (!path) return null;
  const owner = raw.owner || props.albumOwner || "";
  return {
    ...raw,
    path,
    owner,
    name: raw.name || basenameFromPath(path),
    datetime: raw.datetime || raw.date || "",
    date: raw.date || "",
    tags: raw.tags || [],
    rating: raw.rating || 0,
    color_label: raw.color_label || [],
    width: raw.width || 0,
    height: raw.height || 0,
    size: raw.size ?? raw.size_bytes ?? 0,
  };
}

function faceItemToPhoto(item) {
  const resolved = resolveFaceItem(item);
  return normalizeAiPhoto({
    source: resolved?.source,
    path: albumPhotoPath(resolved?.source),
    owner: props.albumOwner,
  });
}

function openFaceDetailPhoto(item) {
  const photo = faceItemToPhoto(item);
  if (!photo) return;
  const navPhotos = (detailItems.value || [])
    .map((entry) => faceItemToPhoto(entry))
    .filter(Boolean);
  emit("open-photo", { photo, navPhotos });
}

function openDuplicatePhoto(photo, group) {
  const normalized = normalizeAiPhoto(photo);
  if (!normalized) return;
  const navPhotos = (group?.photos || [])
    .map((entry) => normalizeAiPhoto(entry))
    .filter(Boolean);
  emit("open-photo", { photo: normalized, navPhotos });
}

function openSearchPhoto(photo) {
  const normalized = normalizeAiPhoto(photo);
  if (!normalized) return;
  const navPhotos = (searchResults.value || [])
    .map((entry) => normalizeAiPhoto(entry))
    .filter(Boolean);
  emit("open-photo", { photo: normalized, navPhotos });
}

function openRename(person) {
  renamePersonId.value = person.id;
  renameValue.value = person.name || "";
  renameOpen.value = true;
}

function saveRename() {
  if (!renamePersonId.value) return;
  emit("patch-person", renamePersonId.value, { name: renameValue.value.trim() });
  renameOpen.value = false;
  if (selectedPerson.value?.id === renamePersonId.value) {
    selectedPerson.value = { ...selectedPerson.value, name: renameValue.value.trim() };
  }
}

function openDelete(person) {
  if (!person?.id) return;
  deletePersonTarget.value = person;
  deleteOpen.value = true;
}

function confirmDelete() {
  const person = deletePersonTarget.value;
  if (!person?.id) return;
  emit("delete-person", person.id);
  deleteOpen.value = false;
  deletePersonTarget.value = null;
  if (selectedPerson.value?.id === person.id) {
    backFromDetail();
  }
}

function startAdopt(cluster) {
  adoptClusterId.value = cluster.cluster_id;
  adoptName.value = "";
}

function confirmAdopt() {
  if (!adoptClusterId.value || !adoptName.value.trim()) return;
  captureListScroll();
  facesSyncBusy.value = true;
  emit("adopt-cluster", {
    cluster_id: adoptClusterId.value,
    name: adoptName.value.trim(),
  });
  adoptClusterId.value = "";
  adoptName.value = "";
  viewingCluster.value = null;
}

watch(
  () => props.busy,
  (busy) => {
    if (facesSyncBusy.value && !busy) {
      facesSyncBusy.value = false;
      restoreListScroll();
    }
  },
);

function mergeIntoPerson(cluster, personId) {
  if (!personId) return;
  captureListScroll();
  emit("merge-cluster", {
    target_person_id: personId,
    cluster_id: cluster.cluster_id,
    center_label: "variant",
  });
  viewingCluster.value = null;
  restoreListScroll();
}

function openPerson(person) {
  selectedPerson.value = person;
  viewingCluster.value = null;
  showOriginalThumbs.value = false;
  showNaturalClusters.value = false;
  viewingPersonCenter.value = null;
}

function openCluster(cluster) {
  captureListScroll();
  viewingCluster.value = cluster;
  selectedPerson.value = null;
  showOriginalThumbs.value = false;
  adoptClusterId.value = "";
  adoptName.value = "";
}

function backFromDetail() {
  selectedPerson.value = null;
  viewingCluster.value = null;
  adoptClusterId.value = "";
  adoptName.value = "";
  showNaturalClusters.value = false;
  viewingPersonCenter.value = null;
  restoreListScroll();
}

function clusterFaces(cluster) {
  if (Array.isArray(cluster?.faces) && cluster.faces.length) {
    return cluster.faces.map((face) => resolveFaceItem(face)).filter(Boolean);
  }
  return (cluster?.face_ids || [])
    .map((face_id) => resolveFaceItem({ face_id }))
    .filter(Boolean);
}

function clusterPreviewFaceIds(cluster) {
  const faces = clusterFaces(cluster);
  const fromFaces = faces.slice(0, 16).map((f) => f.face_id).filter(Boolean);
  if (fromFaces.length) return fromFaces;
  const fromIds = (cluster?.face_ids || []).slice(0, 16).filter(Boolean);
  if (fromIds.length) return fromIds;
  const rep = cluster?.representative_face_id;
  return rep ? [rep] : [];
}

const detailSourceItems = computed(() => {
  if (showNaturalClusters.value && viewingPersonCenter.value?.appearances?.length) {
    return viewingPersonCenter.value.appearances;
  }
  if (selectedPerson.value) {
    return Array.isArray(selectedPerson.value.appearances)
      ? selectedPerson.value.appearances
      : [];
  }
  if (viewingCluster.value) return clusterFaces(viewingCluster.value);
  return [];
});

function detailItemKey(item) {
  if (item?.face_id) return item.face_id;
  if (item?.identity_key) return item.identity_key;
  return `${item?.source || ""}:${(item?.box_xywh || item?.box || []).join(",")}`;
}

function detailItemBox(item) {
  return item?.box_xywh || item?.box || null;
}

function appearanceFaceCropProps(appearance) {
  return {
    album: props.albumName,
    faceId: appearance?.face_id || "",
    source: appearance?.source || "",
    faceBox: detailItemBox(appearance),
  };
}

function clearDetailPaging() {
  detailItems.value = [];
  detailOffset.value = 0;
  detailTotal.value = 0;
  detailLoadingMore.value = false;
}

function loadMoreDetailItems() {
  if (detailLoadingMore.value) return;
  const source = detailSourceItems.value;
  detailTotal.value = source.length;
  if (detailOffset.value > 0 && detailOffset.value >= detailTotal.value) return;
  detailLoadingMore.value = true;
  try {
    const page = source.slice(detailOffset.value, detailOffset.value + DETAIL_PAGE_SIZE);
    if (detailOffset.value === 0) {
      detailItems.value = page;
    } else {
      detailItems.value.push(...page);
    }
    detailOffset.value += page.length;
  } finally {
    detailLoadingMore.value = false;
  }
}

function resetDetailPaging() {
  detailItems.value = [];
  detailOffset.value = 0;
  detailTotal.value = detailSourceItems.value.length;
  loadMoreDetailItems();
}

function handleDetailScrollNearBottom() {
  loadMoreDetailItems();
}

watch(
  () => selectedPerson.value?.id
    || viewingCluster.value?.cluster_id
    || viewingPersonCenter.value?.center_id
    || "",
  (id) => {
    if (id) resetDetailPaging();
    else clearDetailPaging();
  },
);

// After merge/adopt/refresh, restore faces list scroll when still on the overview.
watch(
  () => props.suggestions?.clusters,
  () => {
    if (selectedPerson.value || viewingCluster.value) return;
    restoreListScroll();
  },
  { flush: "post" },
);

// After refresh, keep the same loaded window size against the updated source list.
watch(detailSourceItems, (source) => {
  if (!selectedPerson.value && !viewingCluster.value) return;
  detailTotal.value = source.length;
  const keep = Math.min(Math.max(detailOffset.value, DETAIL_PAGE_SIZE), source.length);
  detailItems.value = source.slice(0, keep);
  detailOffset.value = detailItems.value.length;
});

function applyPersonFilter(person) {
  emit("filter-person", person);
  // Keep modal open — user can still manage faces; album behind updates.
}

async function runTextSearch() {
  const text = String(query.value || "").trim();
  if (!text || !props.onSearchText) return;
  searchLoading.value = true;
  searchError.value = "";
  searchResults.value = [];
  try {
    const data = await props.onSearchText({
      text,
      scope: "current_album",
      minScore: Number(minScore.value) || 0,
      limit: Math.max(1, Math.min(200, Number(limit.value) || 60)),
      inModal: true,
    });
    searchResults.value = data?.results || [];
    if (searchResults.value.length === 0) {
      searchError.value = t("albums.aiSearchEmpty");
    }
  } catch (error) {
    searchError.value = error?.message || t("msg.requestFailed");
  } finally {
    searchLoading.value = false;
  }
}

async function loadDuplicates() {
  if (!props.onLoadDuplicates) return;
  dupLoading.value = true;
  dupError.value = "";
  try {
    const data = await props.onLoadDuplicates();
    dupGroups.value = data?.groups || [];
    dupLoaded.value = true;
    if (dupGroups.value.length === 0) {
      dupError.value = t("albums.aiDuplicatesEmpty");
    }
  } catch (error) {
    dupGroups.value = [];
    dupError.value = error?.message || t("msg.requestFailed");
  } finally {
    dupLoading.value = false;
  }
}
</script>

<template>
  <div class="fixed inset-0 z-[110] flex items-stretch justify-center bg-black/55 p-0 sm:items-center sm:p-4" @click.self="emit('close')">
    <div
      class="flex h-full w-full max-w-6xl flex-col overflow-hidden border shadow-2xl sm:h-[min(92vh,56rem)] sm:rounded-2xl"
      :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'"
    >
      <div class="flex items-center gap-2 border-b px-3 py-2.5" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
        <IconWand class="h-4 w-4 shrink-0 text-amber-500" />
        <p class="min-w-0 flex-1 truncate text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
          {{ t("albums.aiTools") }} · {{ albumName }}
        </p>
        <button
          v-if="section === 'faces'"
          type="button"
          class="rounded p-1.5 transition"
          :class="isDark ? 'text-stone-400 hover:bg-stone-800' : 'text-stone-500 hover:bg-stone-100'"
          :title="t('people.refresh')"
          @click="emit('refresh')"
        >
          <IconRefresh class="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          class="rounded-full p-1.5 transition"
          :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
          @click="emit('close')"
        >
          <IconClose class="h-3.5 w-3.5" />
        </button>
      </div>

      <div class="flex min-h-0 flex-1">
        <nav
          class="flex w-[4.5rem] shrink-0 flex-col gap-1 border-r p-1.5 sm:w-44 sm:p-2"
          :class="isDark ? 'border-stone-700 bg-stone-950/60' : 'border-stone-200 bg-stone-50'"
        >
          <button
            v-for="item in navItems"
            :key="item.id"
            type="button"
            class="flex items-center gap-2 rounded-xl px-2 py-2 text-left text-xs font-medium transition sm:px-2.5"
            :class="section === item.id
              ? (isDark ? 'bg-amber-500/20 text-amber-100' : 'bg-amber-100 text-amber-900')
              : (isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-700 hover:bg-stone-200')"
            @click="section = item.id"
          >
            <component :is="item.icon" class="mx-auto h-4 w-4 shrink-0 sm:mx-0" />
            <span class="hidden truncate sm:inline">{{ t(item.labelKey) }}</span>
          </button>
        </nav>

        <div class="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden p-3 sm:p-4">
          <!-- Adopt / sync: wait for refreshed faces before showing list -->
          <template v-if="section === 'faces' && facesSyncBusy">
            <div class="flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
              <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent" />
              <p class="text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                {{ t("people.loading") }}
              </p>
            </div>
          </template>

          <!-- Person detail: virtual grid + segmented load -->
          <template v-else-if="section === 'faces' && selectedPerson">
            <div class="flex min-h-0 flex-1 flex-col">
              <div class="shrink-0">
                <button
                  type="button"
                  class="mb-3 inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium"
                  :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'"
                  @click="viewingPersonCenter ? backFromPersonCenter() : backFromDetail()"
                >
                  <IconArrowLeft class="h-3.5 w-3.5" />{{
                    viewingPersonCenter ? t("people.backToClusters") : t("people.back")
                  }}
                </button>
                <div class="mb-3 flex flex-wrap items-center gap-2">
                  <h3 class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
                    {{ personLabel(selectedPerson) }}
                  </h3>
                  <button
                    v-if="editable"
                    type="button"
                    class="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-[10px] font-medium"
                    :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'"
                    @click="openRename(selectedPerson)"
                  >
                    <IconPencil class="h-3 w-3" />{{ t("people.rename") }}
                  </button>
                  <button
                    v-if="editable"
                    type="button"
                    class="inline-flex items-center gap-1 rounded-lg px-2 py-1 text-[10px] font-medium"
                    :class="isDark ? 'bg-red-950/50 text-red-300' : 'bg-red-100 text-red-800'"
                    @click="openDelete(selectedPerson)"
                  >
                    <IconTrash class="h-3 w-3" />{{ t("people.delete") }}
                  </button>
                  <button
                    type="button"
                    class="rounded-lg px-2 py-1 text-[10px] font-medium"
                    :class="isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900'"
                    @click="applyPersonFilter(selectedPerson)"
                  >
                    {{ t("albums.aiFilterByPerson") }}
                  </button>
                  <label class="inline-flex items-center gap-1.5 text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                    <input v-model="showOriginalThumbs" type="checkbox" class="rounded accent-amber-500" />
                    {{ t("albums.aiShowOriginalThumbs") }}
                  </label>
                  <label
                    v-if="personHasMultipleCenters && !viewingPersonCenter"
                    class="inline-flex items-center gap-1.5 text-[10px]"
                    :class="isDark ? 'text-stone-400' : 'text-stone-500'"
                  >
                    <input v-model="showNaturalClusters" type="checkbox" class="rounded accent-amber-500" />
                    {{ t("people.showNaturalClusters") }}
                  </label>
                </div>
                <p class="mb-3 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  <template v-if="viewingPersonCenter">
                    {{ centerClusterLabel(viewingPersonCenter) }}
                  </template>
                  <template v-else>
                    {{ t("people.photoCount", { count: selectedPerson.appearance_count ?? detailTotal }) }}
                    <span v-if="personHasMultipleCenters">
                      · {{ selectedPerson.center_count || selectedPerson.centers.length }} {{ t("people.naturalClusterCount") }}
                    </span>
                  </template>
                </p>
              </div>
              <div
                v-if="showNaturalClusters && personHasMultipleCenters && !viewingPersonCenter"
                class="app-scrollbar min-h-0 flex-1 overflow-y-auto"
              >
                <div class="grid grid-cols-1 gap-3 md:grid-cols-2">
                  <article
                    v-for="center in personCentersDisplay"
                    :key="center.center_id"
                    class="relative cursor-pointer rounded-xl border p-3 transition"
                    :class="isDark
                      ? 'border-stone-700 bg-stone-950/40 hover:border-stone-600 hover:bg-stone-900/50'
                      : 'border-stone-200 bg-stone-50 hover:border-stone-300 hover:bg-white'"
                    @click="openPersonCenter(center)"
                  >
                    <button
                      v-if="editable"
                      type="button"
                      class="absolute right-2 top-2 rounded-md p-1 transition"
                      :class="isDark ? 'bg-red-950/50 text-red-300 hover:bg-red-950/70' : 'bg-red-100 text-red-800 hover:bg-red-200'"
                      :title="t('people.removeCluster')"
                      @click.stop="removePersonCluster(center)"
                    >
                      <IconTrash class="h-3.5 w-3.5" />
                    </button>
                    <p class="mb-2 pr-8 text-xs font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-800'">
                      {{ centerClusterLabel(center) }}
                    </p>
                    <div class="flex flex-wrap gap-1.5">
                      <button
                        v-for="appearance in centerPreviewAppearances(center)"
                        :key="detailItemKey(appearance)"
                        type="button"
                        class="h-12 w-12 overflow-hidden rounded-full"
                        :class="isDark ? 'bg-stone-800' : 'bg-stone-200'"
                        @click.stop="openPersonClusterPhoto(center, appearance)"
                      >
                        <FaceAvatar
                          v-if="appearance.source && detailItemBox(appearance)"
                          v-bind="appearanceFaceCropProps(appearance)"
                          :size="48"
                        />
                      </button>
                    </div>
                  </article>
                </div>
              </div>
              <div v-else class="relative min-h-0 flex-1 overflow-hidden">
                <VirtualGrid
                  :items="detailItems"
                  :item-key="detailItemKey"
                  :grid-class="DETAIL_GRID_CLASS"
                  class="absolute inset-0"
                  @scroll-near-bottom="handleDetailScrollNearBottom"
                >
                  <template #default="{ item: appearance }">
                    <article
                      class="group relative cursor-pointer overflow-hidden rounded-xl border"
                      :class="isCurrentCover(appearance)
                        ? (isDark ? 'border-amber-500 ring-1 ring-amber-500/60' : 'border-amber-500 ring-1 ring-amber-400/70')
                        : (isDark ? 'border-stone-700' : 'border-stone-200')"
                      @click="openFaceDetailPhoto(appearance)"
                    >
                      <div class="aspect-square" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                        <LazyImage
                          v-if="showOriginalThumbs"
                          :src="photoThumb(albumPhotoPath(appearance.source), albumOwner)"
                          img-class="h-full w-full object-cover"
                          alt=""
                        />
                        <FaceAvatar
                          v-else-if="appearance.source && detailItemBox(appearance)"
                          v-bind="appearanceFaceCropProps(appearance)"
                          :size="160"
                        />
                      </div>
                      <span
                        v-if="isCurrentCover(appearance)"
                        class="absolute left-1 top-1 inline-flex items-center gap-0.5 rounded-md bg-amber-600/90 px-1 py-0.5 text-[9px] font-medium text-white"
                      >
                        <IconCheck class="h-2.5 w-2.5" />{{ t("people.currentCover") }}
                      </span>
                      <div
                        v-if="editable"
                        class="absolute inset-x-0 bottom-0 flex gap-1 bg-gradient-to-t from-black/70 to-transparent p-1.5 opacity-100 transition md:opacity-0 md:group-hover:opacity-100"
                        @click.stop
                      >
                        <button
                          v-if="!isCurrentCover(appearance)"
                          type="button"
                          class="flex-1 rounded-md bg-white/90 px-1 py-1.5 text-[10px] font-medium text-stone-900 hover:bg-white md:py-0.5 md:text-[9px]"
                          @click="setPersonCover(appearance)"
                        >
                          {{ t("people.setCover") }}
                        </button>
                        <button
                          v-else
                          type="button"
                          class="flex-1 rounded-md bg-white/90 px-1 py-1.5 text-[10px] font-medium text-stone-900 hover:bg-white md:py-0.5 md:text-[9px]"
                          @click="clearPersonCover()"
                        >
                          {{ t("people.clearCover") }}
                        </button>
                      </div>
                    </article>
                  </template>
                </VirtualGrid>
              </div>
            </div>
          </template>

          <!-- Cluster detail: virtual grid + segmented load -->
          <template v-else-if="section === 'faces' && viewingCluster">
            <div class="flex min-h-0 flex-1 flex-col">
              <div class="shrink-0">
                <button
                  type="button"
                  class="mb-3 inline-flex items-center gap-1 rounded-lg px-2 py-1 text-xs font-medium"
                  :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'"
                  @click="backFromDetail"
                >
                  <IconArrowLeft class="h-3.5 w-3.5" />{{ t("albums.aiBackToSuggestions") }}
                </button>
                <div class="mb-3 flex flex-wrap items-center gap-2">
                  <h3 class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
                    {{ viewingCluster.synthetic ? t("albums.aiSingleFace") : t("albums.aiCluster") }}
                  </h3>
                  <template v-if="clusterHasMatchSuggestions(viewingCluster)">
                    <button
                      v-for="suggestion in viewingCluster.match_suggestions"
                      :key="`${suggestion.person_id}:${suggestion.center_id || ''}`"
                      type="button"
                      class="rounded-lg px-2 py-1 text-[10px] font-medium"
                      :class="isDark ? 'bg-sky-900/50 text-sky-100 hover:bg-sky-900/70' : 'bg-sky-100 text-sky-900 hover:bg-sky-200'"
                      @click="mergeIntoPerson(viewingCluster, suggestion.person_id)"
                    >
                      {{ t("albums.aiMatchMerge", { name: suggestion.person_name, percent: matchPercent(suggestion.score) }) }}
                    </button>
                  </template>
                  <template v-if="editable && adoptClusterId === viewingCluster.cluster_id">
                    <input
                      v-model="adoptName"
                      type="text"
                      class="w-28 rounded-lg border px-2 py-1 text-[11px] sm:w-36"
                      :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
                      :placeholder="t('people.namePlaceholder')"
                      @keyup.enter="confirmAdopt"
                    />
                    <button type="button" class="rounded-lg bg-amber-600 px-2 py-1 text-[10px] font-medium text-white" @click="confirmAdopt">
                      <IconCheck class="inline h-3 w-3" /> {{ t("albums.save") }}
                    </button>
                  </template>
                  <button
                    v-else-if="editable"
                    type="button"
                    class="rounded-lg px-2 py-1 text-[10px] font-medium"
                    :class="isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900'"
                    @click="startAdopt(viewingCluster)"
                  >
                    {{ t("albums.aiNameFace") }}
                  </button>
                  <select
                    v-if="editable && namedPeople.length > 0"
                    class="max-w-[8.5rem] rounded-lg border px-1.5 py-1 text-[10px]"
                    :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-200' : 'border-stone-300 bg-white text-stone-800'"
                    @change="(e) => { if (e.target.value) mergeIntoPerson(viewingCluster, e.target.value); e.target.value = ''; }"
                  >
                    <option value="">{{ t("people.merge") }}…</option>
                    <option v-for="person in namedPeople" :key="person.id" :value="person.id">
                      {{ personLabel(person) }}
                    </option>
                  </select>
                  <label class="ml-auto inline-flex items-center gap-1.5 text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                    <input v-model="showOriginalThumbs" type="checkbox" class="rounded accent-amber-500" />
                    {{ t("albums.aiShowOriginalThumbs") }}
                  </label>
                </div>
                <p class="mb-3 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  {{ t("people.photoCount", { count: viewingCluster.face_count || detailTotal }) }}
                </p>
              </div>
              <div v-if="detailTotal === 0" class="rounded-xl border px-3 py-6 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
                {{ t("albums.aiFacePreviewEmpty") }}
              </div>
              <div v-else class="relative min-h-0 flex-1 overflow-hidden">
                <VirtualGrid
                  :items="detailItems"
                  :item-key="detailItemKey"
                  :grid-class="DETAIL_GRID_CLASS"
                  class="absolute inset-0"
                  @scroll-near-bottom="handleDetailScrollNearBottom"
                >
                  <template #default="{ item: face }">
                    <article
                      class="cursor-pointer overflow-hidden rounded-xl border"
                      :class="isDark ? 'border-stone-700' : 'border-stone-200'"
                      @click="openFaceDetailPhoto(face)"
                    >
                      <div class="aspect-square" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                        <LazyImage
                          v-if="showOriginalThumbs && face.source"
                          :src="photoThumb(albumPhotoPath(face.source), albumOwner)"
                          img-class="h-full w-full object-cover"
                          alt=""
                        />
                        <FaceAvatar
                          v-else-if="face.face_id || (face.source && detailItemBox(face))"
                          :album="albumName"
                          :face-id="face.face_id"
                          :source="face.source"
                          :face-box="detailItemBox(face)"
                          :size="160"
                        />
                      </div>
                    </article>
                  </template>
                </VirtualGrid>
              </div>
            </div>
          </template>

          <div
            v-else
            ref="listScrollRef"
            class="app-scrollbar min-h-0 flex-1 overflow-y-auto"
            @scroll="captureListScroll"
          >
          <!-- FACES list -->
          <template v-if="section === 'faces'">
              <p class="mb-3 text-[11px] leading-relaxed" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                {{ t("albums.aiFacesManageHint") }}
              </p>

              <h3 class="mb-2 text-xs font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                {{ t("people.title") }}
                <span class="font-normal normal-case">({{ namedPeople.length }})</span>
              </h3>
              <div v-if="busy && namedPeople.length === 0" class="py-6 text-center text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                {{ t("people.loading") }}
              </div>
              <div v-else-if="namedPeople.length === 0" class="mb-5 rounded-xl border px-3 py-4 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
                {{ t("albums.aiNamedFacesEmpty") }}
              </div>
              <div v-else class="mb-6 grid grid-cols-3 gap-2 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6">
                <div
                  v-for="person in namedPeople"
                  :key="person.id"
                  class="relative flex flex-col items-center gap-1 rounded-xl p-2 text-center transition"
                  :class="isDark ? 'bg-stone-800/50 hover:bg-stone-800' : 'bg-stone-50 hover:bg-stone-100'"
                >
                  <button
                    v-if="editable"
                    type="button"
                    class="absolute right-1 top-1 rounded-md p-1 opacity-70 transition hover:opacity-100"
                    :class="isDark ? 'text-red-300 hover:bg-red-950/40' : 'text-red-700 hover:bg-red-100'"
                    :title="t('people.delete')"
                    @click.stop="openDelete(person)"
                  >
                    <IconTrash class="h-3 w-3" />
                  </button>
                  <button
                    type="button"
                    class="flex w-full flex-col items-center gap-1"
                    @click="openPerson(person)"
                  >
                  <div class="h-14 w-14 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-700' : 'bg-stone-200'">
                    <FaceAvatar
                      v-if="personListCover(person)"
                      :album="albumName"
                      :cover="personListCover(person)"
                      :size="56"
                    />
                  </div>
                  <p class="w-full truncate text-[11px] font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
                    {{ personLabel(person) }}
                  </p>
                  <p class="text-[9px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                    {{ t("people.photoCount", { count: person.appearance_count ?? person.appearances?.length ?? person.visible_photo_count ?? 0 }) }}
                  </p>
                  </button>
                </div>
              </div>

              <div class="mb-2 flex flex-wrap items-center justify-between gap-2">
                <h3 class="text-xs font-bold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  {{ t("albums.aiSuggestions") }}
                  <span class="font-normal normal-case">({{ visibleSuggestions.length }})</span>
                </h3>
                <label class="inline-flex items-center gap-1.5 text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  <input v-model="showSingles" type="checkbox" class="rounded accent-amber-500" />
                  {{ t("albums.aiShowSingles") }}
                </label>
              </div>

              <div v-if="busy && visibleSuggestions.length === 0" class="py-6 text-center text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                {{ t("people.loading") }}
              </div>
              <div v-else-if="visibleSuggestions.length === 0" class="rounded-xl border px-3 py-4 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
                {{ t("albums.aiFacesEmpty") }}
              </div>
              <div v-else class="grid grid-cols-1 gap-3 md:grid-cols-2">
                <button
                  v-for="cluster in visibleSuggestions"
                  :key="cluster.cluster_id"
                  type="button"
                  class="rounded-xl border p-3 text-left transition"
                  :class="isDark ? 'border-stone-700 bg-stone-950/40 hover:bg-stone-900' : 'border-stone-200 bg-stone-50 hover:bg-stone-100'"
                  @click="openCluster(cluster)"
                >
                  <div class="mb-2 flex flex-wrap items-center gap-2">
                    <p class="text-xs font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-800'">
                      {{ cluster.synthetic ? t("albums.aiSingleFace") : t("albums.aiCluster") }}
                      · {{ t("people.photoCount", { count: cluster.face_count || cluster.face_ids?.length || 0 }) }}
                    </p>
                    <span class="text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                      {{ t("albums.aiOpenCluster") }}
                    </span>
                  </div>
                  <div
                    v-if="clusterHasMatchSuggestions(cluster)"
                    class="mb-2 flex flex-wrap gap-1.5"
                    @click.stop
                  >
                    <button
                      v-for="suggestion in cluster.match_suggestions"
                      :key="`${suggestion.person_id}:${suggestion.center_id || ''}`"
                      type="button"
                      class="rounded-full px-2 py-0.5 text-[10px] font-medium"
                      :class="isDark ? 'bg-sky-900/50 text-sky-100 hover:bg-sky-900/70' : 'bg-sky-100 text-sky-900 hover:bg-sky-200'"
                      @click="mergeIntoPerson(cluster, suggestion.person_id)"
                    >
                      {{ t("albums.aiMatchSuggest", { name: suggestion.person_name, percent: matchPercent(suggestion.score) }) }}
                    </button>
                  </div>
                  <div class="flex flex-wrap gap-1.5">
                    <div
                      v-for="faceId in clusterPreviewFaceIds(cluster)"
                      :key="faceId"
                      class="h-12 w-12 overflow-hidden rounded-full"
                      :class="isDark ? 'bg-stone-800' : 'bg-stone-200'"
                    >
                      <FaceAvatar :album="albumName" :face-id="faceId" :size="48" />
                    </div>
                  </div>
                </button>
              </div>
          </template>

          <!-- DUPLICATES (in-modal) -->
          <template v-else-if="section === 'duplicates'">
            <div class="mb-3 flex items-center gap-2">
              <p class="flex-1 text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                {{ t("albums.aiDuplicatesHint") }}
              </p>
              <button
                type="button"
                class="rounded-lg px-2 py-1 text-[10px] font-medium"
                :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'"
                :disabled="dupLoading"
                @click="loadDuplicates"
              >
                <IconRefresh class="inline h-3 w-3" /> {{ t("people.refresh") }}
              </button>
            </div>
            <div v-if="dupLoading" class="py-10 text-center text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
              {{ t("people.loading") }}
            </div>
            <div v-else-if="dupError && dupGroups.length === 0" class="rounded-xl border px-3 py-6 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
              {{ dupError }}
            </div>
            <div v-else class="flex flex-col gap-4">
              <section
                v-for="(group, gi) in dupGroups"
                :key="group.group_key || group.id || gi"
                class="rounded-xl border p-3"
                :class="isDark ? 'border-stone-700' : 'border-stone-200'"
              >
                <div class="mb-2 flex flex-wrap items-center gap-2">
                  <span
                    class="rounded-full px-2 py-0.5 text-[10px] font-semibold"
                    :class="isDark ? 'bg-amber-900/40 text-amber-200' : 'bg-amber-100 text-amber-800'"
                  >
                    {{ group.kind === 'near' ? t('albums.duplicatesNear') : t('albums.duplicatesExact') }}
                  </span>
                  <p class="text-xs font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-800'">
                    {{ t('albums.duplicateGroupPhotos', { count: group.photo_count ?? (group.photos || []).length }) }}
                  </p>
                  <span
                    v-if="group.kind === 'near' && group.max_hamming_distance != null"
                    class="text-[10px]"
                    :class="isDark ? 'text-stone-500' : 'text-stone-400'"
                  >
                    {{ t('albums.duplicatesDistance', { distance: group.max_hamming_distance }) }}
                  </span>
                </div>
                <div class="grid grid-cols-3 gap-2 sm:grid-cols-4 md:grid-cols-5">
                  <div
                    v-for="photo in (group.photos || [])"
                    :key="photo.path || photo.stable_id"
                    class="cursor-pointer overflow-hidden rounded-lg"
                    :class="isDark ? 'bg-stone-800' : 'bg-stone-100'"
                    @click="openDuplicatePhoto(photo, group)"
                  >
                    <div class="aspect-square">
                      <LazyImage
                        :src="photoThumb(photo.path, photo.owner || albumOwner)"
                        img-class="h-full w-full object-cover"
                        alt=""
                      />
                    </div>
                    <p class="truncate px-1 py-0.5 text-[9px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                      {{ photo.name || String(photo.path || "").split("/").pop() }}
                    </p>
                  </div>
                </div>
              </section>
            </div>
          </template>

          <!-- SEARCH (in-modal) -->
          <template v-else-if="section === 'search'">
            <textarea
              v-model="query"
              rows="3"
              :placeholder="t('albums.textSearchPlaceholder')"
              class="mb-3 w-full resize-none rounded-xl border px-3 py-2 text-sm"
              :class="isDark ? 'border-stone-700 bg-stone-950 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
              @keydown.enter.ctrl="runTextSearch"
            />
            <div class="mb-3 flex flex-wrap items-center gap-3 text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
              <label class="flex items-center gap-1.5">
                {{ t("albums.aiMinScore") }}
                <input v-model.number="minScore" type="number" step="0.01" min="0" max="1" class="w-16 rounded border px-1.5 py-1" :class="isDark ? 'border-stone-700 bg-stone-800' : 'border-stone-300'" />
              </label>
              <label class="flex items-center gap-1.5">
                {{ t("albums.aiResultLimit") }}
                <input v-model.number="limit" type="number" min="1" max="200" class="w-16 rounded border px-1.5 py-1" :class="isDark ? 'border-stone-700 bg-stone-800' : 'border-stone-300'" />
              </label>
              <button
                type="button"
                class="inline-flex items-center gap-1.5 rounded-xl bg-amber-600 px-4 py-2 text-xs font-medium text-white disabled:opacity-50"
                :disabled="!String(query || '').trim() || searchLoading"
                @click="runTextSearch"
              >
                <IconSearch class="h-3.5 w-3.5" />
                {{ searchLoading ? t("albums.aiSearching") : t("albums.aiRunSearch") }}
              </button>
            </div>

            <div v-if="searchLoading" class="py-10 text-center text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
              {{ t("albums.aiSearching") }}
            </div>
            <div v-else-if="searchError && searchResults.length === 0" class="rounded-xl border px-3 py-6 text-center text-[11px]" :class="isDark ? 'border-stone-800 text-stone-500' : 'border-stone-200 text-stone-400'">
              {{ searchError }}
            </div>
            <div v-else-if="searchResults.length" class="grid grid-cols-3 gap-2 sm:grid-cols-4 md:grid-cols-5">
              <article
                v-for="photo in searchResults"
                :key="photo.path || photo.stable_id"
                class="cursor-pointer overflow-hidden rounded-xl border"
                :class="isDark ? 'border-stone-700' : 'border-stone-200'"
                @click="openSearchPhoto(photo)"
              >
                <div class="aspect-square" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                  <LazyImage
                    :src="photoThumb(photo.path, photo.owner || albumOwner)"
                    img-class="h-full w-full object-cover"
                    alt=""
                  />
                </div>
                <div class="px-1.5 py-1">
                  <p class="truncate text-[10px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
                    {{ photo.name || String(photo.path || "").split("/").pop() }}
                  </p>
                  <p v-if="photo.score != null" class="text-[9px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                    {{ Number(photo.score).toFixed(3) }}
                  </p>
                </div>
              </article>
            </div>
          </template>
          </div>
        </div>
      </div>
    </div>

    <teleport to="body">
      <div v-if="renameOpen" class="fixed inset-0 z-[120] flex items-center justify-center bg-black/50 p-4" @click.self="renameOpen = false">
        <div class="w-full max-w-md rounded-2xl border p-4 shadow-xl" :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'">
          <h3 class="mb-3 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("people.rename") }}</h3>
          <input
            v-model="renameValue"
            type="text"
            class="mb-4 w-full rounded-lg border px-3 py-2 text-sm"
            :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
            :placeholder="t('people.namePlaceholder')"
          />
          <div class="flex justify-end gap-2">
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="renameOpen = false">{{ t("albums.cancel") }}</button>
            <button class="rounded-xl bg-amber-600 px-3 py-1.5 text-xs font-medium text-white" @click="saveRename">{{ t("albums.save") }}</button>
          </div>
        </div>
      </div>
    </teleport>

    <teleport to="body">
      <div v-if="deleteOpen && deletePersonTarget" class="fixed inset-0 z-[120] flex items-center justify-center bg-black/50 p-4" @click.self="deleteOpen = false">
        <div class="w-full max-w-md rounded-2xl border p-4 shadow-xl" :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'">
          <h3 class="mb-2 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t("people.delete") }}</h3>
          <p class="mb-4 text-xs leading-relaxed" :class="isDark ? 'text-stone-400' : 'text-stone-600'">
            {{ t("people.deleteConfirm", { name: personLabel(deletePersonTarget) }) }}
          </p>
          <div class="flex justify-end gap-2">
            <button class="rounded-xl px-3 py-1.5 text-xs font-medium" :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="deleteOpen = false">{{ t("albums.cancel") }}</button>
            <button class="rounded-xl bg-red-600 px-3 py-1.5 text-xs font-medium text-white" @click="confirmDelete">{{ t("people.delete") }}</button>
          </div>
        </div>
      </div>
    </teleport>
  </div>
</template>
