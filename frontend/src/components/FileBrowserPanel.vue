<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { prettyPath } from "../utils/path";
import { isImageEntry, thumbnailUrl } from "../utils/fileBrowser";
import { t } from "../i18n/index.js";
import {
  IconArrowLeft, IconChevronLeft, IconChevronRight, IconClipboardCopy, IconClipboardPaste,
  IconClose, IconDownload, IconFile, IconFolder, IconFolderAdd, IconHome,
  IconPencil, IconRefresh, IconTrash, IconUpload, IconList, IconGrid, IconScissors,
} from "../utils/icons";
import UploadModal from "./UploadModal.vue";
import VirtualGrid from "./VirtualGrid.vue";
import { VList } from "virtua/vue";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  allSelected: { type: Boolean, required: true },
  breadcrumbParts: { type: Array, required: true },
  busyList: { type: Boolean, required: true },
  busyUpload: { type: Boolean, required: true },
  canGoUp: { type: Boolean, required: true },
  canGoBack: { type: Boolean, default: false },
  canGoForward: { type: Boolean, default: false },
  clipboardCount: { type: Number, default: 0 },
  clipboardMode: { type: String, default: "copy" },
  currentPath: { type: String, required: true },
  entries: { type: Array, required: true },
  entryTotal: { type: Number, default: 0 },
  isSelected: { type: Function, required: true },
  selectedEntriesCount: { type: Number, required: true },
  selectAllIndeterminate: { type: Boolean, required: true },
  formatSize: { type: Function, required: true },
  formatTime: { type: Function, required: true },
  viewMode: { type: String, required: true },
  embedded: { type: Boolean, default: false },
});

const emit = defineEmits([
  "cancel-clipboard", "clear-selection", "copy-selected", "cut-selected", "delete-entry",
  "delete-selected", "download-entry", "go-up", "go-back", "go-forward", "navigate", "new-folder",
  "open-entry", "paste-here", "refresh", "rename-entry", "set-view-mode",
  "set-selection", "toggle-select-all", "toggle-selection", "upload-directory", "upload-files", "load-more",
  "entry-menu",
]);

const selectAllCheckbox = ref(null);
const showUploadModal = ref(false);
const listSurfaceRef = ref(null);
const entryRefs = new Map();
const isMobile = ref(false);
const longPressTimer = ref(null);
const actionMenuEntry = ref(null);
const dragSelect = reactive({
  active: false,
  moved: false,
  startX: 0,
  startY: 0,
  x: 0,
  y: 0,
  w: 0,
  h: 0,
});
const suppressOpen = ref(false);
const scrollLoadTimer = ref(null);
const listVirtualRef = ref(null);
let entryPress = null;
let suppressTimer = 0;

const showMobileCheckboxes = computed(() => isMobile.value && props.selectedEntriesCount > 0);
const clipboardActionLabel = computed(() => (props.clipboardMode === "cut" ? t("files.clipboardCut") : t("files.clipboardCopy")));
const fileGridClass = computed(() => (
  props.embedded
    ? "grid grid-cols-3 gap-1.5 px-3 py-3"
    : "grid grid-cols-2 gap-3 p-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-7"
));

function onListVirtualScroll(offset) {
  if (scrollLoadTimer.value) return;
  const vl = listVirtualRef.value;
  if (!vl || props.busyList) return;
  let distToBottom = 0;
  try {
    distToBottom = (vl.scrollSize ?? 0) - offset - (vl.viewportSize ?? 0);
  } catch {
    return;
  }
  if (distToBottom < 500) {
    scrollLoadTimer.value = true;
    setTimeout(() => { scrollLoadTimer.value = null; }, 300);
    emit("load-more");
  }
}

function onGridScrollNearBottom() {
  if (!props.busyList) emit("load-more");
}

function updateViewportMode() {
  isMobile.value = props.embedded || window.innerWidth < 768;
}

function entryTooltip(entry) {
  const fullPath = prettyPath(entry.fullPath);
  const type = entry.is_dir ? t("files.directory") : t("files.file");
  const size = props.formatSize(entry.size);
  const modified = props.formatTime(entry.modified);
  return `${t("tooltip.name")}: ${entry.name}\n${t("tooltip.fullPath")}: ${fullPath}\n${t("tooltip.type")}: ${type}\n${t("tooltip.size")}: ${size}\n${t("tooltip.modified")}: ${modified}`;
}

function setEntryRef(path, el) {
  if (el) {
    entryRefs.set(path, el);
  } else {
    entryRefs.delete(path);
  }
}

function armSuppressOpen() {
  suppressOpen.value = true;
  window.clearTimeout(suppressTimer);
  suppressTimer = window.setTimeout(() => {
    suppressOpen.value = false;
  }, 600);
}

let swallowClickTimer = 0;
function swallowNextClick() {
  const stop = (event) => {
    event.preventDefault();
    event.stopPropagation();
    cleanup();
  };
  const cleanup = () => {
    window.removeEventListener("click", stop, true);
    window.clearTimeout(swallowClickTimer);
  };
  window.addEventListener("click", stop, true);
  swallowClickTimer = window.setTimeout(cleanup, 700);
}

function clearEntryPress() {
  window.removeEventListener("pointermove", onEntryPressMove);
  window.removeEventListener("pointerup", onEntryPressEnd);
  window.removeEventListener("pointercancel", onEntryPressCancel);
  entryPress = null;
}

function startLongPress() {
  if (!isMobile.value) {
    return;
  }
  cancelLongPress();
  longPressTimer.value = window.setTimeout(() => {
    longPressTimer.value = null;
    const press = entryPress;
    if (!press || press.moved) return;
    press.menu = true;
    armSuppressOpen();
    swallowNextClick();
    if (props.embedded) emit("entry-menu", press.entry);
    else actionMenuEntry.value = press.entry;
  }, 450);
}

function cancelLongPress() {
  if (longPressTimer.value) {
    clearTimeout(longPressTimer.value);
    longPressTimer.value = null;
  }
}

function closeActionMenu() {
  actionMenuEntry.value = null;
  cancelLongPress();
  window.clearTimeout(suppressTimer);
  suppressOpen.value = false;
}

function onEntryPointerDown(entry, index, event) {
  if (event.button != null && event.button !== 0) return;
  const target = event.target;
  if (target instanceof Element && target.closest("button, input, label, a")) return;
  clearEntryPress();
  entryPress = {
    entry,
    index,
    x: event.clientX,
    y: event.clientY,
    moved: false,
    menu: false,
  };
  startLongPress();
  if (!isMobile.value) return;
  window.addEventListener("pointermove", onEntryPressMove);
  window.addEventListener("pointerup", onEntryPressEnd);
  window.addEventListener("pointercancel", onEntryPressCancel);
}

function onEntryPressMove(event) {
  if (!entryPress || entryPress.moved) return;
  const deltaX = Math.abs(event.clientX - entryPress.x);
  const deltaY = Math.abs(event.clientY - entryPress.y);
  if (deltaX > 12 || deltaY > 12) {
    entryPress.moved = true;
    cancelLongPress();
  }
}

function onEntryPressCancel() {
  clearEntryPress();
  cancelLongPress();
}

function onEntryPressEnd() {
  const press = entryPress;
  clearEntryPress();
  cancelLongPress();
  if (!press || press.moved || press.menu || !isMobile.value) return;
  armSuppressOpen();
  handleEntryClick(press.entry, press.index, press, true);
}

function openMobileAction(action) {
  const entry = actionMenuEntry.value;
  if (!entry) {
    return;
  }
  if (action === "select") {
    emit("set-selection", [entry.fullPath]);
  } else if (action === "cut") {
    emit("set-selection", [entry.fullPath]);
    emit("cut-selected");
  } else if (action === "rename") {
    emit("rename-entry", entry);
  } else if (action === "download") {
    emit("download-entry", entry);
  } else if (action === "delete") {
    emit("delete-entry", entry);
  }
  closeActionMenu();
}

function onSurfacePointerDown(event) {
  if (isMobile.value) {
    return;
  }
  if (event.button !== 0) {
    return;
  }
  const target = event.target;
  if (!(target instanceof HTMLElement)) {
    return;
  }
  if (target.closest("button, input, label")) {
    return;
  }
  if (target.closest("[data-entry-card='true']")) {
    return;
  }
  const surface = listSurfaceRef.value;
  if (!surface) {
    return;
  }

  const rect = surface.getBoundingClientRect();
  dragSelect.active = true;
  dragSelect.moved = false;
  dragSelect.startX = event.clientX;
  dragSelect.startY = event.clientY;
  dragSelect.x = event.clientX - rect.left;
  dragSelect.y = event.clientY - rect.top;
  dragSelect.w = 0;
  dragSelect.h = 0;
}

function onSurfacePointerMove(event) {
  if (isMobile.value) {
    return;
  }
  if (!dragSelect.active) {
    return;
  }
  const surface = listSurfaceRef.value;
  if (!surface) {
    return;
  }
  const rect = surface.getBoundingClientRect();
  const curX = event.clientX;
  const curY = event.clientY;
  const x1 = Math.min(dragSelect.startX, curX);
  const y1 = Math.min(dragSelect.startY, curY);
  const x2 = Math.max(dragSelect.startX, curX);
  const y2 = Math.max(dragSelect.startY, curY);
  dragSelect.x = x1 - rect.left;
  dragSelect.y = y1 - rect.top;
  dragSelect.w = x2 - x1;
  dragSelect.h = y2 - y1;
  dragSelect.moved = dragSelect.w > 6 || dragSelect.h > 6;

  if (!dragSelect.moved) {
    return;
  }

  const selected = [];
  for (const entry of props.entries) {
    const node = entryRefs.get(entry.fullPath);
    if (!node) {
      continue;
    }
    const box = node.getBoundingClientRect();
    if (x1 < box.right && x2 > box.left && y1 < box.bottom && y2 > box.top) {
      selected.push(entry.fullPath);
    }
  }
  emit("set-selection", selected);
}

function onSurfacePointerUp() {
  if (isMobile.value) {
    return;
  }
  if (!dragSelect.active) {
    return;
  }
  suppressOpen.value = dragSelect.moved;
  dragSelect.active = false;
  dragSelect.moved = false;
  dragSelect.w = 0;
  dragSelect.h = 0;
  if (suppressOpen.value) {
    window.setTimeout(() => {
      suppressOpen.value = false;
    }, 120);
  }
}

function handleEntryClick(entry, index, event, fromPointer = false) {
  if (!fromPointer && suppressOpen.value) {
    suppressOpen.value = false;
    window.clearTimeout(suppressTimer);
    return;
  }
  if (event.shiftKey || event.ctrlKey || event.metaKey) {
    emit("toggle-selection", {
      path: entry.fullPath,
      index,
      shiftKey: event.shiftKey,
      additiveKey: event.ctrlKey || event.metaKey,
      replace: false,
    });
    return;
  }

  if (props.selectedEntriesCount > 0) {
    emit("toggle-selection", {
      path: entry.fullPath,
      index,
      additiveKey: true,
      replace: false,
    });
    return;
  }

  emit("open-entry", entry);
}

onMounted(() => {
  updateViewportMode();
  window.addEventListener("resize", updateViewportMode);
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", updateViewportMode);
  cancelLongPress();
  clearEntryPress();
  window.clearTimeout(suppressTimer);
  window.clearTimeout(swallowClickTimer);
});

onMounted(() => {
  if (selectAllCheckbox.value) {
    selectAllCheckbox.value.indeterminate = props.selectAllIndeterminate;
  }
});

watch(() => props.selectAllIndeterminate, (val) => {
  if (selectAllCheckbox.value) {
    selectAllCheckbox.value.indeterminate = val;
  }
});

watch(() => props.embedded, updateViewportMode);

function openUpload() {
  showUploadModal.value = true;
}

defineExpose({ openUpload, closeActionMenu });
</script>

<template>
  <section
    class="flex h-full min-h-0 flex-col"
    :class="[
      props.isDark ? 'text-stone-100' : '',
      embedded ? '' : 'app-frost app-card rounded-2xl p-4 md:p-6',
    ]"
  >
    <!-- Header + toolbar -->
    <div v-if="!embedded" class="mb-5 flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
      <div>
        <p class="mb-2 text-xs font-semibold uppercase tracking-[0.24em]" :class="props.isDark ? 'text-emerald-300' : 'text-emerald-700'">{{ t('files.title') }}</p>
        <h2 class="font-display text-2xl font-semibold md:text-3xl" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'">{{ t('files.title') }}</h2>
      </div>

      <div class="flex min-h-11 flex-wrap items-center gap-2 xl:justify-end">
        <template v-if="selectedEntriesCount === 0">
          <button class="app-btn app-btn-primary px-3 py-2.5 text-sm" :disabled="!canGoUp || busyList" @click="emit('go-up')">
            <IconArrowLeft class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.goUp') }}</span>
          </button>
          <button class="app-btn app-btn-secondary px-3 py-2.5 text-sm md:px-4" :disabled="busyList" @click="emit('refresh')">
            <IconRefresh class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.refresh') }}</span>
          </button>
          <button class="app-btn app-btn-secondary px-3 py-2.5 text-sm md:px-4" @click="emit('new-folder')">
            <IconFolderAdd class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.newFolder') }}</span>
          </button>
          <button class="app-btn app-btn-secondary px-3 py-2.5 text-sm md:px-4" :disabled="busyUpload" @click="showUploadModal = true">
            <IconUpload class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.upload') }}</span>
          </button>
          <div class="ml-auto inline-flex items-center gap-1 rounded-xl p-1" :class="props.isDark ? 'bg-stone-800/90' : 'bg-stone-200/90'">
            <button
              class="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition"
              :class="viewMode === 'list' ? (props.isDark ? 'bg-stone-100 text-stone-900 shadow-sm' : 'bg-white text-stone-900 shadow-sm') : (props.isDark ? 'text-stone-200 hover:bg-stone-700' : 'text-stone-700 hover:bg-stone-300/70')"
              @click="emit('set-view-mode', 'list')"
            >
              <IconList class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.listView') }}</span>
            </button>
            <button
              class="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition"
              :class="viewMode === 'grid' ? (props.isDark ? 'bg-stone-100 text-stone-900 shadow-sm' : 'bg-white text-stone-900 shadow-sm') : (props.isDark ? 'text-stone-200 hover:bg-stone-700' : 'text-stone-700 hover:bg-stone-300/70')"
              @click="emit('set-view-mode', 'grid')"
            >
              <IconGrid class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.gridView') }}</span>
            </button>
          </div>
        </template>

        <template v-else>
          <span class="mr-auto text-sm font-medium" :class="props.isDark ? 'text-stone-200' : 'text-stone-700'">{{ t('files.selectedCount', { count: selectedEntriesCount }) }}</span>
          <button class="inline-flex items-center gap-1.5 rounded-2xl px-4 py-2.5 text-sm font-medium transition" :class="props.isDark ? 'bg-emerald-800 text-emerald-100 hover:bg-emerald-700' : 'bg-emerald-600 text-white hover:bg-emerald-700'" @click="emit('toggle-select-all')">
            {{ allSelected && entries.length >= entryTotal ? t('files.deselectAll') : t('files.selectAll') }}
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl bg-rose-600 px-4 py-2.5 text-sm font-medium text-white transition hover:bg-rose-700" @click="emit('delete-selected')">
            <IconTrash class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.delete') }}</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl bg-amber-600 px-4 py-2.5 text-sm font-medium text-white transition hover:bg-amber-700" @click="emit('copy-selected')">
            <IconClipboardCopy class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.copy') }}</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl bg-sky-600 px-4 py-2.5 text-sm font-medium text-white transition hover:bg-sky-700" @click="emit('cut-selected')">
            <IconScissors class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.cut') }}</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl px-4 py-2.5 text-sm font-medium transition" :class="props.isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" @click="emit('clear-selection')">
            <IconClose class="h-4 w-4" /> <span class="hidden md:inline">{{ t('files.cancel') }}</span>
          </button>
        </template>
      </div>
    </div>

    <!-- Clipboard paste banner -->
    <div
      v-if="!embedded && clipboardCount > 0"
      class="mb-4 flex flex-col gap-3 rounded-2xl border px-4 py-3 sm:flex-row sm:items-center sm:justify-between"
      :class="props.isDark ? 'border-amber-700 bg-amber-950/30' : 'border-amber-300 bg-amber-50'"
    >
      <div class="flex items-center gap-2 text-sm" :class="props.isDark ? 'text-amber-200' : 'text-amber-800'">
        <IconScissors v-if="clipboardMode === 'cut'" class="h-5 w-5 shrink-0" />
        <IconClipboardCopy v-else class="h-5 w-5 shrink-0" />
        <span>{{ t('files.clipboardBanner', { action: clipboardActionLabel, count: clipboardCount }) }}</span>
      </div>
      <div class="flex w-full gap-2 sm:w-auto">
        <button class="inline-flex items-center gap-1.5 rounded-xl bg-amber-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-amber-700" @click="emit('paste-here')">
          <IconClipboardPaste class="h-4 w-4" /> {{ t('files.pasteHere') }}
        </button>
        <button class="inline-flex items-center gap-1.5 rounded-xl px-4 py-2 text-sm font-medium transition" :class="props.isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="emit('cancel-clipboard')">
          <IconClose class="h-4 w-4" /> {{ t('files.cancel') }}
        </button>
      </div>
    </div>

    <!-- Breadcrumbs -->
    <div v-if="!embedded" class="mb-4 flex flex-wrap items-center gap-2 rounded-2xl px-3 py-2.5 app-frost app-card">
      <button
        type="button"
        class="file-path-nav"
        :class="props.isDark ? 'text-stone-200 hover:bg-stone-800' : 'text-stone-700 hover:bg-stone-200'"
        :disabled="!canGoBack || busyList"
        :title="t('files.back')"
        :aria-label="t('files.back')"
        @click="emit('go-back')"
      >
        <IconChevronLeft class="h-5 w-5" />
      </button>
      <button
        type="button"
        class="file-path-nav"
        :class="props.isDark ? 'text-stone-200 hover:bg-stone-800' : 'text-stone-700 hover:bg-stone-200'"
        :disabled="!canGoForward || busyList"
        :title="t('files.forward')"
        :aria-label="t('files.forward')"
        @click="emit('go-forward')"
      >
        <IconChevronRight class="h-5 w-5" />
      </button>
      <button class="inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-sm font-medium transition" :class="props.isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" @click="emit('navigate', '')">
        <IconHome class="h-4 w-4" /> {{ t('files.root') }}
      </button>
      <template v-for="crumb in breadcrumbParts" :key="crumb.path">
        <span :class="props.isDark ? 'text-stone-500' : 'text-stone-400'">/</span>
        <button class="rounded-full px-3 py-1.5 text-sm font-medium transition" :class="props.isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" @click="emit('navigate', crumb.path)">
          {{ crumb.name }}
        </button>
      </template>
    </div>

    <!-- File list / grid -->
    <div class="min-h-0 flex-1">
      <div
        class="flex h-full min-h-0 flex-col overflow-hidden"
        :class="embedded ? '' : 'rounded-xl app-frost app-card'"
      >
      <!-- List header -->
      <div v-if="viewMode === 'list' && !embedded" class="hidden grid-cols-[auto_minmax(0,2fr)_5rem_6rem_10rem_8rem] items-center gap-3 px-4 py-3 text-xs font-semibold uppercase tracking-[0.18em] md:grid app-surface app-split-b" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">
        <label class="flex items-center justify-center">
          <input
            ref="selectAllCheckbox"
            class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
            :checked="allSelected"
            type="checkbox"
            @change="emit('toggle-select-all')"
          />
        </label>
        <span>{{ t('files.name') }}</span>
        <span>{{ t('files.type') }}</span>
        <span>{{ t('files.size') }}</span>
        <span>{{ t('files.modified') }}</span>
        <span class="text-right">{{ t('files.actions') }}</span>
      </div>

      <!-- 外层容器用于 drag-select 事件捕获，不自身滚动 -->
        <div
          ref="listSurfaceRef"
          class="relative min-h-0 flex-1 overflow-hidden"
          style="touch-action: pan-y;"
          :class="dragSelect.active ? 'select-none' : ''"
          @pointerdown="onSurfacePointerDown"
          @pointermove="onSurfacePointerMove"
          @pointerup="onSurfacePointerUp"
          @pointerleave="onSurfacePointerUp"
        >
      <div v-if="busyList && entries.length === 0" class="px-4 py-12 text-center text-sm" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('files.loading') }}</div>
      <div v-else-if="!busyList && entries.length === 0" class="px-4 py-12 text-center text-sm" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('files.empty') }}</div>

      <VList
        v-else-if="viewMode === 'list'"
        ref="listVirtualRef"
        :data="entries"
        class="app-scrollbar absolute inset-0"
        :bufferSize="400"
        @scroll="onListVirtualScroll"
      >
        <template #default="{ item: entry, index }">
        <article
          :ref="(el) => setEntryRef(entry.fullPath, el)"
          data-entry-card="true"
          class="group cursor-pointer select-none border-t px-4 py-3 transition first:border-t-0"
          :class="[
            isSelected(entry.fullPath)
              ? (props.isDark ? 'border-stone-700 bg-teal-500/10' : 'border-stone-200 bg-teal-50')
              : (props.isDark ? 'border-stone-800' : 'border-stone-200'),
          ]"
          :title="embedded || isMobile ? undefined : entryTooltip(entry)"
          @click="handleEntryClick(entry, index, $event)"
          @pointerdown="onEntryPointerDown(entry, index, $event)"
          @contextmenu.prevent
        >
          <div v-if="!embedded" class="hidden gap-3 md:grid md:grid-cols-[auto_minmax(0,2fr)_5rem_6rem_10rem_8rem] md:items-center">
            <label class="hidden items-center justify-center md:flex" @click.stop>
              <input
                class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
                :checked="isSelected(entry.fullPath)"
                type="checkbox"
                @change="emit('toggle-selection', { path: entry.fullPath, index, additiveKey: true })"
              />
            </label>

            <div class="flex min-w-0 items-center gap-3">
              <span class="relative flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded-xl" :class="entry.is_dir ? (props.isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (props.isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
                <img
                  v-if="isImageEntry(entry)"
                  class="pointer-events-none absolute inset-0 h-full w-full object-cover"
                  :src="thumbnailUrl(entry, 96)"
                  loading="lazy"
                  decoding="async"
                  fetchpriority="low"
                  alt=""
                />
                <IconFolder v-if="entry.is_dir" class="h-5 w-5" />
                <IconFile v-else-if="!isImageEntry(entry)" class="h-5 w-5" />
              </span>
              <span class="min-w-0 truncate text-sm font-medium" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'" :title="entry.name">{{ entry.name }}</span>
            </div>

            <span class="hidden text-sm md:block" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ entry.is_dir ? t('files.directory') : t('files.file') }}</span>
            <span class="hidden text-sm md:block" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ formatSize(entry.size) }}</span>
            <span class="hidden text-sm md:block" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ formatTime(entry.modified) }}</span>

            <div class="hidden gap-1.5 opacity-0 transition-opacity group-hover:opacity-100 md:flex md:justify-end" @click.stop>
              <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800" :title="t('files.rename')" @click="emit('rename-entry', entry)">
                <IconPencil class="h-4 w-4" />
              </button>
              <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800 disabled:cursor-not-allowed disabled:opacity-50" :disabled="entry.is_dir" :title="t('files.download')" @click="emit('download-entry', entry)">
                <IconDownload class="h-4 w-4" />
              </button>
              <button class="flex h-8 w-8 items-center justify-center rounded-full text-rose-500 transition hover:bg-rose-100 hover:text-rose-700" :title="t('files.delete')" @click="emit('delete-entry', entry)">
                <IconTrash class="h-4 w-4" />
              </button>
            </div>
          </div>

          <div v-if="embedded" class="flex items-start gap-3">
            <label v-if="showMobileCheckboxes" class="mt-2 flex items-center justify-center" @click.stop>
              <input
                class="h-4 w-4 rounded border-stone-300 text-teal-600 focus:ring-teal-200"
                :checked="isSelected(entry.fullPath)"
                type="checkbox"
                @change="emit('toggle-selection', { path: entry.fullPath, index, additiveKey: true })"
              />
            </label>
            <span class="relative flex h-12 w-12 shrink-0 items-center justify-center overflow-hidden rounded-xl" :class="entry.is_dir ? (props.isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (props.isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
              <img
                v-if="isImageEntry(entry)"
                class="pointer-events-none absolute inset-0 h-full w-full object-cover"
                :src="thumbnailUrl(entry, 96)"
                loading="lazy"
                decoding="async"
                fetchpriority="low"
                alt=""
              />
              <IconFolder v-if="entry.is_dir" class="h-5 w-5" />
              <IconFile v-else-if="!isImageEntry(entry)" class="h-5 w-5" />
            </span>
            <div class="min-w-0 flex-1">
              <p class="truncate text-sm font-medium" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'">{{ entry.name }}</p>
              <p class="mt-1 truncate text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ entry.is_dir ? t('files.directory') : t('files.file') }}</p>
              <p class="mt-1 text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ formatSize(entry.size) }}</p>
            </div>
          </div>
          <div v-else class="md:hidden">
            <div class="flex items-start justify-between gap-3">
              <div class="flex min-w-0 items-center gap-3">
                <label v-if="showMobileCheckboxes" class="flex items-center justify-center" @click.stop>
                  <input
                    class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
                    :checked="isSelected(entry.fullPath)"
                    type="checkbox"
                    @change="emit('toggle-selection', { path: entry.fullPath, index, additiveKey: true })"
                  />
                </label>
                <span class="relative flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-xl" :class="entry.is_dir ? (props.isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (props.isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
                  <img
                    v-if="isImageEntry(entry)"
                    class="pointer-events-none absolute inset-0 h-full w-full object-cover"
                    :src="thumbnailUrl(entry, 96)"
                    loading="lazy"
                    decoding="async"
                    fetchpriority="low"
                    alt=""
                  />
                  <IconFolder v-if="entry.is_dir" class="h-5 w-5" />
                  <IconFile v-else-if="!isImageEntry(entry)" class="h-5 w-5" />
                </span>
                <span class="min-w-0 truncate text-sm font-medium" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'">{{ entry.name }}</span>
              </div>
              <span class="shrink-0 text-sm" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ formatSize(entry.size) }}</span>
            </div>
            <div class="mt-2 flex items-center justify-between text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">
              <span>{{ formatTime(entry.modified) }}</span>
              <span>{{ entry.is_dir ? t('files.directory') : t('files.file') }}</span>
            </div>
          </div>
        </article>
        </template>
      </VList>

      <VirtualGrid
        v-else-if="viewMode === 'grid' && entries.length > 0"
        :items="entries"
        item-key="fullPath"
        :grid-class="fileGridClass"
        class="absolute inset-0"
        @scroll-near-bottom="onGridScrollNearBottom"
      >
        <template #default="{ item: entry, index }">
        <article
          :key="entry.fullPath"
          :ref="(el) => setEntryRef(entry.fullPath, el)"
          data-entry-card="true"
          class="group relative cursor-pointer select-none rounded-2xl border p-3 transition"
          :class="[
            isSelected(entry.fullPath)
              ? (props.isDark ? 'border-teal-500 bg-teal-500/10' : 'border-teal-300 bg-teal-50')
              : embedded
                ? (props.isDark ? 'border-stone-700 bg-stone-900/80' : 'border-stone-200 bg-white/70')
                : 'app-surface app-card',
          ]"
          :title="embedded || isMobile ? undefined : entryTooltip(entry)"
          @click="handleEntryClick(entry, index, $event)"
          @pointerdown="onEntryPointerDown(entry, index, $event)"
          @contextmenu.prevent
        >
          <label class="absolute left-2 top-2 z-10 items-center justify-center rounded-md p-1 shadow-sm" :class="[props.isDark ? 'bg-stone-800/90' : 'bg-white/90', showMobileCheckboxes ? 'flex md:flex' : 'hidden md:flex']" @click.stop>
            <input
              class="h-4 w-4 rounded border-stone-300 text-teal-600 focus:ring-teal-200"
              :checked="isSelected(entry.fullPath)"
              type="checkbox"
              @change="emit('toggle-selection', { path: entry.fullPath, index, additiveKey: true })"
            />
          </label>

          <div
            class="relative flex items-center justify-center overflow-hidden rounded-xl"
            :class="[
              embedded ? 'h-28' : 'h-24',
              entry.is_dir ? (props.isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (props.isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600'),
            ]"
          >
            <img
              v-if="isImageEntry(entry)"
              class="pointer-events-none absolute inset-0 h-full w-full object-cover"
              :src="thumbnailUrl(entry, 256)"
              loading="lazy"
              decoding="async"
              fetchpriority="low"
              alt=""
            />
            <IconFolder v-if="entry.is_dir" class="h-10 w-10" />
            <IconFile v-else-if="!isImageEntry(entry)" class="h-10 w-10" />
          </div>

          <div class="mt-3 min-w-0">
            <p class="truncate text-sm font-semibold" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'" :title="entry.name">{{ entry.name }}</p>
            <template v-if="embedded">
              <p class="mt-1 truncate text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ entry.is_dir ? t('files.directory') : t('files.file') }}</p>
              <p class="mt-1 truncate text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ formatSize(entry.size) }}</p>
            </template>
            <template v-else>
              <p class="mt-1 text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ entry.is_dir ? t('files.directory') : formatSize(entry.size) }}</p>
              <p class="mt-1 text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ formatTime(entry.modified) }}</p>
            </template>
          </div>

          <div class="mt-3 hidden gap-1.5 opacity-0 transition-opacity group-hover:opacity-100 md:flex" @click.stop>
            <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800" :title="t('files.rename')" @click="emit('rename-entry', entry)">
              <IconPencil class="h-4 w-4" />
            </button>
            <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800 disabled:cursor-not-allowed disabled:opacity-50" :disabled="entry.is_dir" :title="t('files.download')" @click="emit('download-entry', entry)">
              <IconDownload class="h-4 w-4" />
            </button>
            <button class="flex h-8 w-8 items-center justify-center rounded-full text-rose-500 transition hover:bg-rose-100 hover:text-rose-700" :title="t('files.delete')" @click="emit('delete-entry', entry)">
              <IconTrash class="h-4 w-4" />
            </button>
          </div>
        </article>
        </template>
      </VirtualGrid>

      <div
        v-if="dragSelect.active && dragSelect.moved"
        class="pointer-events-none absolute z-20 rounded-md border border-sky-500/70 bg-sky-400/15"
        :style="{ left: `${dragSelect.x}px`, top: `${dragSelect.y}px`, width: `${dragSelect.w}px`, height: `${dragSelect.h}px` }"
      ></div>
        </div>
      </div>
    </div>

    <div v-if="!embedded && actionMenuEntry" class="fixed inset-0 z-40 bg-black/35 md:hidden" @click="closeActionMenu()">
      <div class="absolute left-1/2 top-1/2 w-[min(22rem,calc(100%-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-[1.75rem] p-4 shadow-2xl" :class="props.isDark ? 'bg-stone-900 text-stone-100' : 'bg-white text-stone-900'" @click.stop>
        <div class="mb-3 flex items-center justify-between">
          <div class="min-w-0">
            <p class="truncate text-base font-semibold">{{ actionMenuEntry.name }}</p>
            <p class="mt-1 truncate text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ prettyPath(actionMenuEntry.fullPath) }}</p>
          </div>
          <button class="inline-flex h-9 w-9 items-center justify-center rounded-full" :class="props.isDark ? 'bg-stone-800' : 'bg-stone-200'" @click="closeActionMenu()">
            <IconClose class="h-4 w-4" />
          </button>
        </div>
        <div class="grid gap-2">
          <button class="rounded-2xl px-4 py-3 text-left text-sm font-medium" :class="props.isDark ? 'bg-stone-800' : 'bg-stone-100'" @click="openMobileAction('select')">{{ t('files.multiSelect') }}</button>
          <button class="rounded-2xl px-4 py-3 text-left text-sm font-medium" :class="props.isDark ? 'bg-sky-900/70 text-sky-100' : 'bg-sky-100 text-sky-700'" @click="openMobileAction('cut')">{{ t('files.cut') }}</button>
          <button class="rounded-2xl px-4 py-3 text-left text-sm font-medium" :class="props.isDark ? 'bg-stone-800' : 'bg-stone-100'" @click="openMobileAction('rename')">{{ t('files.rename') }}</button>
          <button v-if="!actionMenuEntry.is_dir" class="rounded-2xl px-4 py-3 text-left text-sm font-medium" :class="props.isDark ? 'bg-stone-800' : 'bg-stone-100'" @click="openMobileAction('download')">{{ t('files.download') }}</button>
          <button class="rounded-2xl bg-rose-600 px-4 py-3 text-left text-sm font-medium text-white" @click="openMobileAction('delete')">{{ t('files.delete') }}</button>
        </div>
      </div>
    </div>

    <UploadModal
      :show="showUploadModal"
      :busy-upload="busyUpload"
      :is-dark="props.isDark"
      @close="showUploadModal = false"
      @upload-directory="(files) => { emit('upload-directory', files); showUploadModal = false; }"
      @upload-files="(files) => { emit('upload-files', files); showUploadModal = false; }"
    />
  </section>
</template>