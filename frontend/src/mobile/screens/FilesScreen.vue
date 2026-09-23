<script setup>
import { computed, inject, nextTick, onMounted, ref, watch } from "vue";
import FileBrowserPanel from "../../components/FileBrowserPanel.vue";
import { t } from "../../i18n/index.js";
import { prettyPath } from "../../utils/path";
import {
  IconCheck, IconChevronLeft, IconChevronRight, IconClipboardCopy, IconClose, IconDots,
  IconFolderAdd, IconGrid, IconHome, IconList, IconPencil, IconRefresh, IconScissors, IconTrash, IconUpload,
} from "../../utils/icons";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import Screen from "../layouts/Screen.vue";
import ActionSheet from "../sheets/ActionSheet.vue";

defineOptions({ name: "FilesScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const filesRef = ref(null);
const pathScrollRef = ref(null);
const moreOpen = ref(false);
const entryMenu = ref(null);

const pathLabel = computed(() => prettyPath(app.currentPath) || t("files.root"));
const folderTitle = computed(() => {
  const parts = app.breadcrumbParts || [];
  return parts.length ? parts[parts.length - 1].name : t("files.root");
});
const selecting = computed(() => app.selectedEntries.length > 0);

const moreActions = computed(() => [
  { id: "refresh", label: t("files.refresh"), icon: IconRefresh },
  { id: "new-folder", label: t("files.newFolder"), icon: IconFolderAdd },
  { id: "select", label: t("files.multiSelect"), icon: IconCheck },
  { id: "view", label: app.viewMode === "grid" ? t("files.listView") : t("files.gridView"), icon: app.viewMode === "grid" ? IconList : IconGrid },
]);

const entryActions = computed(() => {
  const entry = entryMenu.value;
  if (!entry) return [];
  const items = [
    { id: "select", label: t("files.multiSelect"), icon: IconCheck },
    { id: "cut", label: t("files.cut"), icon: IconScissors },
    { id: "rename", label: t("files.rename"), icon: IconPencil },
  ];
  if (!entry.is_dir) items.push({ id: "download", label: t("files.download") });
  items.push({ id: "delete", label: t("files.delete"), danger: true, icon: IconTrash });
  return items;
});

function scrollPathToEnd() {
  nextTick(() => {
    requestAnimationFrame(() => {
      const el = pathScrollRef.value;
      if (!el) return;
      el.scrollLeft = el.scrollWidth;
    });
  });
}

function onMore(action) {
  moreOpen.value = false;
  if (action.id === "refresh") app.refreshList();
  else if (action.id === "new-folder") app.promptNewFolder();
  else if (action.id === "select" && app.entries[0]) app.setSelection([app.entries[0].fullPath]);
  else if (action.id === "view") app.setViewMode(app.viewMode === "grid" ? "list" : "grid");
}

function onEntryAction(action) {
  const entry = entryMenu.value;
  entryMenu.value = null;
  if (!entry) return;
  if (action.id === "select") app.setSelection([entry.fullPath]);
  else if (action.id === "cut") {
    app.setSelection([entry.fullPath]);
    app.cutSelected();
  } else if (action.id === "rename") app.renameEntry(entry);
  else if (action.id === "download") app.downloadEntry(entry);
  else if (action.id === "delete") app.deleteEntry(entry);
}

watch(() => app.currentPath, scrollPathToEnd);

onMounted(() => {
  app.refreshList();
  scrollPathToEnd();
});
</script>

<template>
  <Screen>
    <template #bar>
      <header class="mobile-topbar">
        <template v-if="selecting">
          <div class="min-w-0 flex-1">
            <h1 class="truncate text-base font-semibold">{{ t("files.selectedCount", { count: app.selectedEntries.length }) }}</h1>
          </div>
          <button type="button" class="mobile-icon-btn" @click="app.copySelected()"><IconClipboardCopy class="h-5 w-5" /></button>
          <button type="button" class="mobile-icon-btn" @click="app.cutSelected()"><IconScissors class="h-5 w-5" /></button>
          <button type="button" class="mobile-icon-btn is-danger" @click="app.deleteSelected()"><IconTrash class="h-5 w-5" /></button>
          <button type="button" class="mobile-icon-btn" @click="app.clearSelection()"><IconClose class="h-5 w-5" /></button>
        </template>
        <template v-else>
          <div class="min-w-0 flex-1">
            <p class="text-[10px] font-semibold uppercase tracking-[0.22em]" :class="ui.isDark ? 'text-teal-300' : 'text-teal-700'">{{ t("nav.files") }}</p>
            <h1 class="truncate text-base font-semibold">{{ folderTitle }}</h1>
          </div>
          <button
            type="button"
            class="mobile-icon-btn"
            :title="app.viewMode === 'grid' ? t('files.listView') : t('files.gridView')"
            @click="app.setViewMode(app.viewMode === 'grid' ? 'list' : 'grid')"
          >
            <IconList v-if="app.viewMode === 'grid'" class="h-5 w-5" />
            <IconGrid v-else class="h-5 w-5" />
          </button>
          <button type="button" class="mobile-icon-btn is-accent" :disabled="app.busy.upload" @click="filesRef?.openUpload()">
            <IconUpload class="h-5 w-5" />
          </button>
          <button type="button" class="mobile-icon-btn" @click="moreOpen = true">
            <IconDots class="h-5 w-5" />
          </button>
        </template>
      </header>
    </template>

    <div
      class="flex shrink-0 items-center gap-1 border-b px-3 py-2"
      :class="ui.isDark ? 'border-stone-700' : 'border-stone-200'"
    >
      <button
        type="button"
        class="mobile-icon-btn shrink-0"
        :disabled="!app.canGoBack || app.busy.list"
        :title="t('files.back')"
        @click="app.goBack()"
      >
        <IconChevronLeft class="h-5 w-5" />
      </button>
      <button
        type="button"
        class="mobile-icon-btn shrink-0"
        :disabled="!app.canGoForward || app.busy.list"
        :title="t('files.forward')"
        @click="app.goForward()"
      >
        <IconChevronRight class="h-5 w-5" />
      </button>
      <div ref="pathScrollRef" class="mobile-path-scroll app-scrollbar min-w-0 flex-1">
        <div class="flex w-max items-center gap-1 text-sm">
          <button
            type="button"
            class="inline-flex items-center gap-1 rounded-full px-3 py-1.5 font-medium transition"
            :class="ui.isDark ? 'bg-stone-800 text-stone-100' : 'bg-stone-100 text-stone-800'"
            @click="app.navigateTo('')"
          >
            <IconHome class="h-4 w-4" />
            {{ t("files.root") }}
          </button>
          <template v-for="crumb in app.breadcrumbParts" :key="crumb.path">
            <IconChevronRight class="h-4 w-4 shrink-0" :class="ui.isDark ? 'text-stone-500' : 'text-stone-400'" />
            <button
              type="button"
              class="rounded-full px-3 py-1.5 font-medium whitespace-nowrap transition"
              :class="crumb.path === app.currentPath
                ? (ui.isDark ? 'bg-teal-500/20 text-teal-200' : 'bg-teal-100 text-teal-800')
                : (ui.isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-100 text-stone-700')"
              @click="app.navigateTo(crumb.path)"
            >
              {{ crumb.name }}
            </button>
          </template>
        </div>
      </div>
    </div>

    <div
      v-if="app.clipboard.length > 0"
      class="flex shrink-0 items-center gap-2 border-b px-4 py-2 text-sm"
      :class="ui.isDark ? 'border-stone-700 bg-amber-950/50 text-amber-200' : 'border-stone-200 bg-amber-50 text-amber-800'"
    >
      <span class="min-w-0 flex-1 truncate">{{ t("files.clipboardBanner", { action: app.clipboardMode === "cut" ? t("files.clipboardCut") : t("files.clipboardCopy"), count: app.clipboard.length }) }}</span>
      <button type="button" class="rounded-full px-2.5 py-1 font-medium" @click="app.pasteHere()">{{ t("files.pasteHere") }}</button>
      <button type="button" class="mobile-press-soft inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-full" @click="app.cancelClipboard()"><IconClose class="h-4 w-4" /></button>
    </div>

    <FileBrowserPanel
      ref="filesRef"
      embedded
      class="min-h-0 flex-1"
      :is-dark="ui.isDark"
      :all-selected="app.allSelected"
      :breadcrumb-parts="app.breadcrumbParts"
      :busy-list="app.busy.list"
      :busy-upload="app.busy.upload"
      :can-go-up="app.canGoUp"
      :can-go-back="app.canGoBack"
      :can-go-forward="app.canGoForward"
      :clipboard-count="app.clipboard.length"
      :clipboard-mode="app.clipboardMode"
      :current-path="app.currentPath"
      :entries="app.entries"
      :entry-total="app.entryTotal"
      :format-size="app.formatSize"
      :format-time="app.formatTime"
      :is-selected="app.isSelected"
      :select-all-indeterminate="app.selectAllIndeterminate"
      :selected-entries-count="app.selectedEntries.length"
      :view-mode="app.viewMode"
      @cancel-clipboard="app.cancelClipboard"
      @clear-selection="app.clearSelection"
      @copy-selected="app.copySelected"
      @cut-selected="app.cutSelected"
      @delete-entry="app.deleteEntry"
      @delete-selected="app.deleteSelected"
      @download-entry="app.downloadEntry"
      @go-up="app.goUp"
      @load-more="app.loadMoreEntries"
      @navigate="app.navigateTo"
      @new-folder="app.promptNewFolder"
      @open-entry="app.openEntry"
      @paste-here="app.pasteHere"
      @refresh="app.refreshList"
      @rename-entry="app.renameEntry"
      @set-view-mode="app.setViewMode"
      @set-selection="app.setSelection"
      @toggle-select-all="app.toggleSelectAll"
      @toggle-selection="app.toggleSelection"
      @upload-directory="(files) => app.uploadFiles(files, true)"
      @upload-files="(files) => app.uploadFiles(files, false)"
      @entry-menu="entryMenu = $event"
    />

    <ActionSheet :show="moreOpen" :title="pathLabel" :actions="moreActions" @close="moreOpen = false" @select="onMore" />
    <ActionSheet
      :show="!!entryMenu"
      :title="entryMenu?.name"
      :subtitle="entryMenu ? prettyPath(entryMenu.fullPath) : ''"
      :actions="entryActions"
      @close="entryMenu = null"
      @select="onEntryAction"
    />
  </Screen>
</template>
