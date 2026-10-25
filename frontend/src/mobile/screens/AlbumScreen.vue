<script setup>
import { computed, inject, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { t } from "../../i18n/index.js";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import { albumAiLocation, albumViewLocation } from "../router.js";
import { discardOverlayStack, currentHistoryIsOverlay } from "../useOverlayHistory.js";
import Screen from "../layouts/Screen.vue";
import AlbumEngine from "../AlbumEngine.vue";
import ActionSheet from "../sheets/ActionSheet.vue";
import AppSlider from "../../components/AppSlider.vue";
import {
  THUMB_DEFAULT,
  THUMB_DETAILS_MIN,
  THUMB_MAX,
} from "../../utils/photoCardLayout.js";
import {
  IconAlertCircle, IconArrowLeft, IconCamera, IconCheck, IconClose, IconDatabase, IconDots, IconDownload, IconTrash,
  IconFilter, IconGear, IconImage, IconImport, IconList, IconRefresh, IconSearch,
  IconSort, IconWand, IconZoomIn, IconZoomOut,
} from "../../utils/icons";

defineOptions({ name: "AlbumScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const route = useRoute();
const router = useRouter();

const searchOpen = ref(false);
const searchActive = ref(false);
const searchInputRef = ref(null);
const moreOpen = ref(false);
const selectMenuOpen = ref(false);
const aiPickerOpen = ref(false);
const searchText = ref("");
const sheetThumbSize = ref(THUMB_DEFAULT);
const sheetCompact = ref(false);

const dirName = computed(() => {
  const raw = route.params.dirName;
  if (Array.isArray(raw)) return raw.filter(Boolean).join("/");
  return String(raw || "");
});
const albumObj = computed(() =>
  app.albums.find((a) => a.dir_name === dirName.value)
  || app.sharedAlbums.find((a) => a.dir_name === dirName.value)
);
const title = computed(() => albumObj.value?.name || dirName.value);
const isShared = computed(() => dirName.value.includes("/"));
const panel = computed(() => ui.albumPanelRef);
const filterDot = computed(() => !!panel.value?.hasActiveFilters);
const sortDot = computed(() => !!panel.value?.hasActiveSort);
const filterOpen = computed(() => !!panel.value?.filterPanelOpen);
const sortOpen = computed(() => !!panel.value?.sortPanelOpen);
const selecting = computed(() => !!panel.value?.selectMode && !panel.value?.inSpecialView);
const selectedCount = computed(() => Number(panel.value?.selectedPhotoCount || 0));
const selectActions = computed(() => [
  {
    id: "export",
    label: t("albums.exportSelected"),
    icon: IconDownload,
    disabled: selectedCount.value === 0 || !!panel.value?.exportSelectionPending,
  },
  {
    id: "download",
    label: t("albums.downloadSelected"),
    icon: IconDownload,
    disabled: selectedCount.value === 0 || !!panel.value?.downloadSelectionPending,
  },
  {
    id: "delete",
    label: t("albums.deleteSelected"),
    icon: IconTrash,
    danger: true,
    disabled: selectedCount.value === 0 || !panel.value?.currentAlbumCanDelete,
  },
]);

watch(selecting, (on) => {
  if (!on) {
    selectMenuOpen.value = false;
    return;
  }
  moreOpen.value = false;
  aiPickerOpen.value = false;
  if (searchOpen.value) closeSearch();
});

const moreActions = computed(() => {
  const p = panel.value;
  const items = [];
  if (!isShared.value && !p?.inSpecialView) {
    items.push({ id: "import", label: t("albums.import"), icon: IconImport });
  }
  if (!p?.inSpecialView) {
    items.push({ id: "select", label: t("albums.select"), icon: IconCheck });
  }
  items.push({ id: "rebuild", label: t("albums.rebuildIndex"), icon: IconDatabase });
  if (p?.inSpecialView) {
    items.push({ id: "photos", label: t("albums.backToPhotos"), icon: IconImage });
  }
  items.push({ id: "ai", label: t("albums.aiTools"), icon: IconWand });
  if (!p?.inSpecialView) {
    items.push({ id: "view", label: t("view.settings"), icon: IconGear });
  }
  return items;
});

const thumbnailSize = computed({
  get: () => sheetThumbSize.value,
  set: (value) => {
    const n = Number(value);
    sheetThumbSize.value = n;
    panel.value?.setThumbnailSize?.(n);
  },
});
const viewCompact = computed(() => sheetCompact.value);
const forcedThumbsOnly = computed(() => sheetThumbSize.value < THUMB_DETAILS_MIN);

watch(moreOpen, (open) => {
  if (!open) return;
  sheetThumbSize.value = Number(panel.value?.thumbnailSize ?? THUMB_DEFAULT);
  sheetCompact.value = !!panel.value?.viewCompact;
});

async function syncAlbum() {
  if (route.name !== "album" || !dirName.value) return;
  if (app.currentAlbum !== dirName.value) {
    await app.selectAlbum(dirName.value);
  }
  const photo = route.query.photo;
  if (photo && panel.value) {
    panel.value.openPhotoDetail({ path: String(photo) });
  }
}

watch(() => [route.name, dirName.value, route.query.photo], () => { syncAlbum(); }, { immediate: true });

function goBack() {
  discardOverlayStack();
  moreOpen.value = false;
  aiPickerOpen.value = false;
  clearSearch();
  if (currentHistoryIsOverlay() || typeof history.state?.back !== "string") {
    router.replace({ name: "albums" });
    return;
  }
  ui.markPushClose?.();
  router.back();
}

function openSearch() {
  searchOpen.value = true;
  const input = searchInputRef.value;
  input?.closest("form")?.classList.remove("mobile-search-standby");
  input?.focus({ preventScroll: true });
}

function clearSearch() {
  searchText.value = "";
  searchOpen.value = false;
  searchActive.value = false;
  app.clearMobileSearch?.();
  searchInputRef.value?.blur();
}

function closeSearch() {
  clearSearch();
}

function submitSearch() {
  const text = searchText.value.trim();
  if (!text || !dirName.value) return;
  searchActive.value = true;
  searchInputRef.value?.blur();
  app.searchAlbumByText({ text, page: true });
}

function onSelectAction(action) {
  selectMenuOpen.value = false;
  if (action?.id === "export") panel.value?.openExportSelection?.();
  if (action?.id === "download") panel.value?.downloadSelected?.();
  if (action?.id === "delete") panel.value?.confirmDeleteSelected?.();
}

function onMore(action) {
  moreOpen.value = false;
  window.setTimeout(() => applyMoreAction(action), 40);
}

const aiToolActions = computed(() => [
  { id: "faces", label: t("albums.aiFaces"), hint: t("albums.aiFacesPick"), icon: IconCamera },
  { id: "duplicates", label: t("albums.aiDuplicates"), hint: t("albums.aiDuplicatesPick"), icon: IconWand },
  { id: "search", label: t("albums.aiTextSearch"), hint: t("albums.aiSearchPick"), icon: IconSearch },
]);

function applyMoreAction(action) {
  if (action.id === "ai") {
    window.setTimeout(() => { aiPickerOpen.value = true; }, 40);
    app.refreshPeopleForAlbum?.();
    return;
  }
  if (action.id === "view") {
    const dir = dirName.value;
    if (!dir) return;
    window.setTimeout(() => {
      if (route.name !== "album") return;
      router.push(albumViewLocation(dir));
    }, 260);
    return;
  }
  const p = panel.value;
  if (!p) return;
  if (action.id === "import") p.openImportModal();
  else if (action.id === "select") p.toggleSelectMode();
  else if (action.id === "rebuild") app.rebuildAlbumIndex();
  else if (action.id === "photos") app.showAlbumPhotos();
}

function onSelectAiTool(action) {
  const id = action?.id || "faces";
  const dir = dirName.value;
  if (!dir) return;
  aiPickerOpen.value = false;
  window.setTimeout(() => {
    if (route.name !== "album") return;
    router.push(albumAiLocation(dir, id));
  }, 260);
}

function toggleFilter() {
  panel.value?.toggleFilterPanel();
}

function toggleSort() {
  panel.value?.toggleSortPanel();
}

function setViewCompact(compact) {
  if (!compact && sheetThumbSize.value < THUMB_DETAILS_MIN) {
    sheetThumbSize.value = THUMB_DETAILS_MIN;
  }
  sheetCompact.value = compact;
  panel.value?.setViewCompact(compact);
}

function resetThumbnailSize() {
  sheetThumbSize.value = THUMB_DEFAULT;
  panel.value?.resetThumbnailSize();
}
</script>

<template>
  <Screen class="h-full">
    <template #bar>
      <header class="mobile-topbar">
        <template v-if="selecting">
          <div class="min-w-0 flex-1">
            <h1 class="truncate text-base font-semibold">{{ t("albums.selectedPhotos", { count: selectedCount }) }}</h1>
          </div>
          <button type="button" class="mobile-icon-btn" :title="t('albums.deselectAll')" @click="panel?.clearSelection()">
            <IconClose class="h-5 w-5" />
          </button>
          <button type="button" class="mobile-icon-btn" :title="t('albums.selectAll')" @click="panel?.selectAllPhotos()">
            <IconCheck class="h-5 w-5" />
          </button>
          <button type="button" class="mobile-icon-btn" @click="selectMenuOpen = true">
            <IconDots class="h-5 w-5" />
          </button>
        </template>
        <template v-else>
        <button type="button" class="mobile-icon-btn" @click="goBack">
          <IconArrowLeft class="h-5 w-5" />
        </button>
        <div class="relative flex min-w-0 flex-1 items-center">
          <div v-show="!searchOpen" class="min-w-0 flex-1">
            <h1 class="truncate text-base font-semibold">{{ title }}</h1>
            <p v-if="isShared" class="truncate text-[0.6875rem] text-blue-500">{{ t("albums.from", { owner: albumObj?.owner }) }}</p>
          </div>
          <form
            class="flex min-w-0 flex-1 items-center gap-2"
            :class="searchOpen ? '' : 'mobile-search-standby'"
            @submit.prevent="submitSearch"
          >
            <input
              ref="searchInputRef"
              v-model="searchText"
              type="text"
              enterkeyhint="search"
              autocomplete="off"
              class="mobile-search-input"
              :placeholder="t('albums.textSearchPlaceholder')"
              @keydown.enter.prevent="submitSearch"
            />
            <button v-show="searchOpen" type="submit" class="mobile-icon-btn">
              <IconSearch class="h-5 w-5" />
            </button>
          </form>
        </div>
        <button v-if="!searchOpen" type="button" class="mobile-icon-btn" @click="openSearch">
          <IconSearch class="h-5 w-5" />
        </button>
        <button v-else type="button" class="mobile-icon-btn" @click="closeSearch">
          <IconClose class="h-5 w-5" />
        </button>
        <button
          v-if="!panel?.inSpecialView"
          type="button"
          class="mobile-icon-btn relative"
          :class="filterOpen || filterDot ? 'is-accent' : ''"
          :title="t('albums.filters')"
          @click="toggleFilter"
        >
          <IconFilter class="h-5 w-5" />
          <span v-if="filterDot" class="absolute right-1.5 top-1.5 h-1.5 w-1.5 rounded-full bg-amber-500"></span>
        </button>
        <button
          v-if="!panel?.inSpecialView"
          type="button"
          class="mobile-icon-btn relative"
          :class="sortOpen || sortDot ? 'is-accent' : ''"
          :title="t('albums.sort')"
          @click="toggleSort"
        >
          <IconSort class="h-5 w-5" />
          <span v-if="sortDot" class="absolute right-1.5 top-1.5 h-1.5 w-1.5 rounded-full bg-amber-500"></span>
        </button>
        <button type="button" class="mobile-icon-btn" @click="moreOpen = true">
          <IconDots class="h-5 w-5" />
        </button>
        </template>
      </header>
    </template>
    <AlbumEngine class="min-h-0 flex-1" :searching="searchActive" />
    <ActionSheet
      :show="selectMenuOpen"
      :title="t('albums.selectedPhotos', { count: selectedCount })"
      :actions="selectActions"
      @close="selectMenuOpen = false"
      @select="onSelectAction"
    />
    <ActionSheet
      :show="moreOpen"
      :title="title"
      :actions="moreActions"
      @close="moreOpen = false"
      @select="onMore"
    >
      <template #default="{ pick }">
        <div
          v-if="!panel?.inSpecialView"
          class="mobile-sheet-view"
          @pointerdown.stop
        >
          <p class="mobile-sheet-view-label">{{ t("albums.thumbnailSize") }}</p>
          <div class="flex items-center gap-2">
            <IconZoomOut class="h-4 w-4 shrink-0 opacity-50" />
            <AppSlider
              v-model="thumbnailSize"
              :min="0"
              :max="THUMB_MAX"
              :tick="THUMB_DETAILS_MIN"
              :tick-title="t('view.detailsMinTick')"
              :is-dark="ui.isDark"
              :aria-label="t('albums.thumbnailSize')"
            />
            <IconZoomIn class="h-4 w-4 shrink-0 opacity-50" />
            <button type="button" class="mobile-sheet-view-reset" @click="resetThumbnailSize">
              <IconRefresh class="h-4 w-4" />
            </button>
          </div>
          <div class="mobile-sheet-segment">
            <button
              type="button"
              class="mobile-sheet-segment-btn"
              :class="{ 'is-active': !viewCompact }"
              :title="forcedThumbsOnly ? t('view.detailsTooSmall') : undefined"
              @click="setViewCompact(false)"
            >
              <IconList class="h-4 w-4" />
              {{ t("view.details") }}
              <IconAlertCircle
                v-if="forcedThumbsOnly"
                class="absolute -right-1 -top-1 h-3.5 w-3.5 text-amber-400"
              />
            </button>
            <button
              type="button"
              class="mobile-sheet-segment-btn"
              :class="{ 'is-active': viewCompact }"
              @click="setViewCompact(true)"
            >
              <IconImage class="h-4 w-4" />
              {{ t("view.thumbsOnly") }}
            </button>
          </div>
        </div>
        <button
          v-for="action in moreActions"
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
      </template>
    </ActionSheet>
    <ActionSheet
      :show="aiPickerOpen"
      :title="t('albums.aiTools')"
      :subtitle="t('albums.aiChooseTool')"
      @close="aiPickerOpen = false"
    >
      <button
        v-for="item in aiToolActions"
        :key="item.id"
        type="button"
        class="mobile-sheet-action"
        @click="onSelectAiTool(item)"
      >
        <component :is="item.icon" class="h-5 w-5 shrink-0 opacity-80" />
        <span class="min-w-0 flex-1 text-left">
          <span class="block truncate">{{ item.label }}</span>
          <span class="mt-0.5 block text-[11px] font-normal opacity-50">{{ item.hint }}</span>
        </span>
      </button>
    </ActionSheet>
  </Screen>
</template>
