<script setup>
import { inject, computed, ref, watch } from "vue";
import { useRoute } from "vue-router";
import AlbumPanel from "../components/AlbumPanel.vue";
import { t } from "../i18n/index.js";
import { partitionFilterClauses } from "../utils/filterDefs.js";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "./context.js";

const props = defineProps({
  searching: { type: Boolean, default: false },
});

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const route = useRoute();
const panelRef = ref(null);

// Cover pages stay above the album. Keep this grid mounted so scroll survives the return.
const gridPaused = computed(() => route.name !== "album" && route.name !== "album-ai" && route.name !== "album-face" && route.name !== "album-face-center" && route.name !== "album-view");

watch(panelRef, (el) => {
  if (ui) ui.albumPanelRef = el;
}, { immediate: true });

function sortValue(photo, field) {
  switch (field) {
    case "date": return String(photo.datetime || photo.date || "");
    case "name": return String(photo.name || "");
    case "size": return Number(photo.size_bytes || photo.size || 0);
    case "rating": return Number(photo.rating || 0);
    case "camera": return String(photo.camera || "");
    case "lens": return String(photo.lens || "");
    default: return "";
  }
}

const shownPhotos = computed(() => {
  if (!props.searching) return app.albumPhotos;
  const list = [...(app.mobileSearchPhotos || [])];
  const levels = app.albumSort || [];
  if (!levels.length) return list;
  list.sort((a, b) => {
    for (const level of levels) {
      const av = sortValue(a, level.field);
      const bv = sortValue(b, level.field);
      const cmp = typeof av === "number" && typeof bv === "number"
        ? av - bv
        : String(av).localeCompare(String(bv));
      if (cmp !== 0) return level.dir === "desc" ? -cmp : cmp;
    }
    return 0;
  });
  return list;
});

function onApplyFilters() {
  if (!props.searching) {
    app.refreshAlbumPhotos();
    return;
  }
  const text = String(app.mobileSearchText || "").trim();
  if (!text) return;
  const { metadata } = partitionFilterClauses(app.albumFilters?.clauses || []);
  app.searchAlbumByText({ text, page: true, filters: metadata });
}

function onUpdateSort(next) {
  app.albumSort = next;
  if (props.searching) return;
  app.refreshAlbumPhotos();
}

function onLoadMore() {
  if (props.searching) return;
  app.loadMoreAlbumPhotos();
}

function onShowPhotos() {
  if (props.searching) return;
  app.showAlbumPhotos();
}

function onShowDuplicates() {
  if (props.searching) return;
  app.showAlbumDuplicates();
}

function onSearchText(payload) {
  if (props.searching) return;
  app.searchAlbumByText(payload);
}

async function onDeletePhotos(paths) {
  await app.deleteAlbumPhotos(paths);
  if (!props.searching) return;
  const drop = new Set(paths || []);
  app.mobileSearchPhotos = (app.mobileSearchPhotos || []).filter((photo) => !drop.has(photo.path));
}

watch(gridPaused, (paused) => {
  if (!paused) return;
  const panel = panelRef.value;
  if (!panel) return;
  panel.closeDetail?.({ force: true });
  if (panel.filterSortPanelOpen) panel.filterSortPanelOpen = false;
  if (panel.filterPanelOpen) panel.filterPanelOpen = false;
  if (panel.sortPanelOpen) panel.sortPanelOpen = false;
  if (panel.aiToolsPanelOpen) panel.aiToolsPanelOpen = false;
  if (panel.aiToolsPickerOpen) panel.aiToolsPickerOpen = false;
});

function onDetailPhoto(photo) {
  ui.viewerPhoto = photo || null;
  const p = panelRef.value;
  const i = p?.detailIndex ?? -1;
  const list = p?.detailPhotoList || [];
  if (!photo) {
    ui.viewerHasPrev = false;
    ui.viewerHasNext = false;
    ui.viewerPrevPhoto = null;
    ui.viewerNextPhoto = null;
    ui.viewerIndexLabel = "";
    return;
  }
  ui.viewerHasPrev = !!p?.hasPrev;
  ui.viewerHasNext = !!p?.hasNext;
  ui.viewerIndexLabel = i >= 0 && list.length ? `${i + 1} / ${list.length}` : "";
  ui.viewerPrevPhoto = i > 0 ? list[i - 1] : null;
  ui.viewerNextPhoto = i >= 0 && i < list.length - 1 ? list[i + 1] : null;
}
</script>

<template>
  <AlbumPanel
    ref="panelRef"
    embedded
    :grid-paused="gridPaused"
    :is-dark="ui.isDark"
    :busy-albums="searching ? app.mobileSearchLoading : app.busy.albums"
    :albums="app.albums"
    :shared-albums="app.sharedAlbums"
    :current-album="app.currentAlbum"
    :current-user="app.currentUser"
    :album-view-mode="searching ? 'photos' : app.albumViewMode"
    :album-photos="shownPhotos"
    :duplicate-groups="app.albumDuplicateGroups"
    :duplicate-total="app.albumDuplicateTotal"
    :similar-results="searching ? [] : app.aiSimilarResults"
    :similar-query="searching ? null : app.aiSimilarQuery"
    :similar-loading="searching ? false : app.aiSimilarLoading"
    :album-people="app.albumPeople"
    :album-people-loading="app.albumPeopleLoading"
    :album-face-hints-loading="app.albumFaceHintsLoading"
    :album-face-suggestions="app.albumFaceSuggestionsDisplay"
    :album-faces-editable="app.albumFacesEditable"
    :album-faces-owner="app.albumFacesOwner"
    :available-tags="app.albumAvailableTags"
    :available-cameras="app.albumAvailableCameras"
    :available-lenses="app.albumAvailableLenses"
    :available-formats="app.albumAvailableFormats"
    :date-histogram="app.dateHistogram"
    :filters="app.albumFilters"
    :empty-label="searching ? t('albums.noTextResults') : ''"
    :sort="app.albumSort"
    :format-size="app.formatSize"
    :format-time="app.formatTime"
    :fetch-list="app.fetchFilesForPicker"
    :load-all-photos="app.loadAllRemainingAlbumPhotos"
    :collections="app.collections"
    :selected-collection-id="app.selectedCollectionId"
    :on-create-album="app.createAlbum"
    :on-update-album="app.updateAlbum"
    :on-share-album="app.shareAlbum"
    :on-unshare-album="app.unshareAlbum"
    :on-search-text-in-modal="(opts) => app.searchAlbumByText({ ...opts, inModal: true })"
    :on-load-duplicates-in-modal="app.loadAlbumDuplicatesForModal"
    :fetch-photos-by-paths="app.fetchAlbumPhotosByPaths"
    @create-album="app.createAlbum"
    @refresh-albums="app.refreshAlbums"
    @select-album="app.selectAlbum"
    @update-album="app.updateAlbum"
    @delete-album="app.deleteAlbum"
    @delete-photos="onDeletePhotos"
    @share-album="app.shareAlbum"
    @show-duplicates="onShowDuplicates"
    @show-photos="onShowPhotos"
    @find-similar="app.findSimilarPhotos"
    @search-text="onSearchText"
    @refresh-album-people="() => app.refreshPeopleForAlbum()"
    @patch-album-person="(id, updates) => app.patchAlbumPerson(id, updates)"
    @remove-album-center="(body) => app.removeAlbumPersonCenter(body)"
    @delete-album-person="(id) => app.deleteAlbumPerson(id)"
    @adopt-album-cluster="app.adoptAlbumCluster"
    @merge-album-cluster="app.mergeAlbumCluster"
    @unshare-album="app.unshareAlbum"
    @apply-filters="onApplyFilters"
    @update-sort="onUpdateSort"
    @import-selected="app.importSelectedToAlbum"
    @import-local="(files) => app.importLocalToAlbum(files)"
    @import-cloud="(paths) => app.importCloudToAlbum(paths)"
    @update-photo-meta="app.updatePhotoMeta"
    @copy-photo-to-album="app.copyPhotoToAlbum"
    @rebuild-index="app.rebuildAlbumIndex"
    @load-more-photos="onLoadMore"
    @select-collection="app.selectCollection"
    @detail-photo="onDetailPhoto"
  />
</template>
