<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watchEffect } from "vue";
import { prettyPath } from "../utils/path";
import {
  IconArrowLeft, IconClipboardCopy, IconClipboardPaste,
  IconClose, IconDownload, IconFile, IconFolder, IconFolderAdd,
  IconHome, IconPencil, IconRefresh, IconTrash, IconUpload, IconList, IconGrid,
} from "../utils/icons";
import UploadModal from "./UploadModal.vue";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  allSelected: { type: Boolean, required: true },
  breadcrumbParts: { type: Array, required: true },
  busyList: { type: Boolean, required: true },
  busyUpload: { type: Boolean, required: true },
  canGoUp: { type: Boolean, required: true },
  clipboardCount: { type: Number, default: 0 },
  currentPath: { type: String, required: true },
  entries: { type: Array, required: true },
  isSelected: { type: Function, required: true },
  selectedEntriesCount: { type: Number, required: true },
  selectAllIndeterminate: { type: Boolean, required: true },
  formatSize: { type: Function, required: true },
  formatTime: { type: Function, required: true },
  viewMode: { type: String, required: true },
});

const emit = defineEmits([
  "cancel-clipboard", "clear-selection", "copy-selected", "delete-entry",
  "delete-selected", "download-entry", "go-up", "navigate", "new-folder",
  "open-entry", "paste-here", "refresh", "rename-entry", "set-view-mode",
  "set-selection", "toggle-select-all", "toggle-selection", "upload-directory", "upload-files",
]);

const selectAllCheckbox = ref(null);
const showUploadModal = ref(false);
const listSurfaceRef = ref(null);
const entryRefs = new Map();
const isMobile = ref(false);
const longPressTimer = ref(null);
const actionMenuEntry = ref(null);
const longPressPoint = reactive({ x: 0, y: 0 });
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

const currentPathLabel = computed(() => prettyPath(props.currentPath));
const showMobileCheckboxes = computed(() => isMobile.value && props.selectedEntriesCount > 0);

function updateViewportMode() {
  isMobile.value = window.innerWidth < 768;
}

function entryTooltip(entry) {
  const fullPath = prettyPath(entry.fullPath);
  const type = entry.is_dir ? "目录" : "文件";
  const size = props.formatSize(entry.size);
  const modified = props.formatTime(entry.modified);
  return `名称: ${entry.name}\n完整路径: ${fullPath}\n类型: ${type}\n大小: ${size}\n修改时间: ${modified}`;
}

function setEntryRef(path, el) {
  if (el) {
    entryRefs.set(path, el);
  } else {
    entryRefs.delete(path);
  }
}

function startLongPress(entry) {
  if (!isMobile.value) {
    return;
  }
  cancelLongPress();
  longPressTimer.value = window.setTimeout(() => {
    longPressTimer.value = null;
    actionMenuEntry.value = entry;
    suppressOpen.value = true;
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
  window.setTimeout(() => {
    suppressOpen.value = false;
  }, 0);
}

function onEntryPointerDown(entry, event) {
  longPressPoint.x = event.clientX;
  longPressPoint.y = event.clientY;
  startLongPress(entry);
}

function onEntryPointerMove(event) {
  if (!longPressTimer.value) {
    return;
  }
  const deltaX = Math.abs(event.clientX - longPressPoint.x);
  const deltaY = Math.abs(event.clientY - longPressPoint.y);
  if (deltaX > 8 || deltaY > 8) {
    cancelLongPress();
  }
}

function openMobileAction(action) {
  const entry = actionMenuEntry.value;
  if (!entry) {
    return;
  }
  if (action === "select") {
    emit("set-selection", [entry.fullPath]);
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

function openEntryGuard(entry) {
  if (suppressOpen.value) {
    return;
  }
  emit("open-entry", entry);
}

function handleEntryClick(entry, index, event) {
  if (isMobile.value && props.selectedEntriesCount > 0) {
    emit("toggle-selection", {
      path: entry.fullPath,
      index,
      additiveKey: true,
      replace: false,
    });
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
  openEntryGuard(entry);
}

onMounted(() => {
  updateViewportMode();
  window.addEventListener("resize", updateViewportMode);
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", updateViewportMode);
  cancelLongPress();
});

watchEffect(() => {
  if (selectAllCheckbox.value) {
    selectAllCheckbox.value.indeterminate = props.selectAllIndeterminate;
  }
});
</script>

<template>
  <section
    class="flex h-full min-h-0 flex-col rounded-[2rem] border p-4 shadow-[0_22px_60px_rgba(71,43,17,0.16)] backdrop-blur md:p-6"
    :class="props.isDark ? 'border-stone-700 bg-stone-900/70 text-stone-100' : 'border-white/70 bg-white/70'"
  >
    <!-- Header + toolbar -->
    <div class="mb-5 flex flex-col gap-4 xl:flex-row xl:items-start xl:justify-between">
      <div>
        <p class="mb-2 text-xs font-semibold uppercase tracking-[0.24em]" :class="props.isDark ? 'text-emerald-300' : 'text-emerald-700'">文件浏览</p>
        <h2 class="font-display text-2xl font-semibold md:text-3xl" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'">{{ currentPathLabel }}</h2>
      </div>

      <div class="flex min-h-11 flex-wrap items-center gap-2 xl:justify-end">
        <template v-if="selectedEntriesCount === 0">
          <button class="inline-flex items-center gap-1.5 rounded-2xl px-3 py-2.5 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-60" :class="props.isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'" :disabled="!canGoUp || busyList" @click="emit('go-up')">
            <IconArrowLeft class="h-4 w-4" /> <span class="hidden md:inline">返回上级</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl px-3 py-2.5 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-60 md:px-4" :class="props.isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" :disabled="busyList" @click="emit('refresh')">
            <IconRefresh class="h-4 w-4" /> <span class="hidden md:inline">刷新</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl px-3 py-2.5 text-sm font-medium transition md:px-4" :class="props.isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" @click="emit('new-folder')">
            <IconFolderAdd class="h-4 w-4" /> <span class="hidden md:inline">新建文件夹</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl px-3 py-2.5 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-60 md:px-4" :class="props.isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" :disabled="busyUpload" @click="showUploadModal = true">
            <IconUpload class="h-4 w-4" /> <span class="hidden md:inline">上传</span>
          </button>
          <div class="ml-auto inline-flex items-center gap-1 rounded-xl p-1" :class="props.isDark ? 'bg-stone-800/90' : 'bg-stone-200/90'">
            <button
              class="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition"
              :class="viewMode === 'list' ? (props.isDark ? 'bg-stone-100 text-stone-900 shadow-sm' : 'bg-white text-stone-900 shadow-sm') : (props.isDark ? 'text-stone-200 hover:bg-stone-700' : 'text-stone-700 hover:bg-stone-300/70')"
              @click="emit('set-view-mode', 'list')"
            >
              <IconList class="h-4 w-4" /> <span class="hidden md:inline">列表</span>
            </button>
            <button
              class="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition"
              :class="viewMode === 'grid' ? (props.isDark ? 'bg-stone-100 text-stone-900 shadow-sm' : 'bg-white text-stone-900 shadow-sm') : (props.isDark ? 'text-stone-200 hover:bg-stone-700' : 'text-stone-700 hover:bg-stone-300/70')"
              @click="emit('set-view-mode', 'grid')"
            >
              <IconGrid class="h-4 w-4" /> <span class="hidden md:inline">图标</span>
            </button>
          </div>
        </template>

        <template v-else>
          <span class="mr-auto text-sm font-medium" :class="props.isDark ? 'text-stone-200' : 'text-stone-700'">已选中 {{ selectedEntriesCount }} 项</span>
          <button class="inline-flex items-center gap-1.5 rounded-2xl bg-rose-600 px-4 py-2.5 text-sm font-medium text-white transition hover:-translate-y-0.5 hover:bg-rose-700" @click="emit('delete-selected')">
            <IconTrash class="h-4 w-4" /> <span class="hidden md:inline">删除</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl bg-amber-600 px-4 py-2.5 text-sm font-medium text-white transition hover:-translate-y-0.5 hover:bg-amber-700" @click="emit('copy-selected')">
            <IconClipboardCopy class="h-4 w-4" /> <span class="hidden md:inline">复制</span>
          </button>
          <button class="inline-flex items-center gap-1.5 rounded-2xl px-4 py-2.5 text-sm font-medium transition" :class="props.isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" @click="emit('clear-selection')">
            <IconClose class="h-4 w-4" /> <span class="hidden md:inline">取消</span>
          </button>
        </template>
      </div>
    </div>

    <!-- Clipboard paste banner -->
    <div
      v-if="clipboardCount > 0"
      class="mb-4 flex flex-col gap-3 rounded-2xl border px-4 py-3 sm:flex-row sm:items-center sm:justify-between"
      :class="props.isDark ? 'border-amber-700 bg-amber-950/30' : 'border-amber-300 bg-amber-50'"
    >
      <div class="flex items-center gap-2 text-sm" :class="props.isDark ? 'text-amber-200' : 'text-amber-800'">
        <IconClipboardCopy class="h-5 w-5 shrink-0" />
        <span>已复制 <strong>{{ clipboardCount }}</strong> 项，浏览到目标目录后点击粘贴</span>
      </div>
      <div class="flex w-full gap-2 sm:w-auto">
        <button class="inline-flex items-center gap-1.5 rounded-xl bg-amber-600 px-4 py-2 text-sm font-medium text-white transition hover:bg-amber-700" @click="emit('paste-here')">
          <IconClipboardPaste class="h-4 w-4" /> 粘贴到此处
        </button>
        <button class="inline-flex items-center gap-1.5 rounded-xl px-4 py-2 text-sm font-medium transition" :class="props.isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="emit('cancel-clipboard')">
          <IconClose class="h-4 w-4" /> 取消
        </button>
      </div>
    </div>

    <!-- Breadcrumbs -->
    <div class="mb-4 flex flex-wrap items-center gap-2 rounded-2xl border px-4 py-3" :class="props.isDark ? 'border-stone-700 bg-stone-900/70' : 'border-stone-200/80 bg-white/70'">
      <button class="inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-sm font-medium transition" :class="props.isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'" @click="emit('navigate', '')">
        <IconHome class="h-4 w-4" /> root
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
        class="flex h-full min-h-0 flex-col overflow-hidden rounded-[1.5rem] border"
        :class="props.isDark ? 'border-stone-700 bg-stone-900/60' : 'border-stone-200/80 bg-white/55'"
      >
      <!-- List header -->
      <div v-if="viewMode === 'list'" class="hidden grid-cols-[auto_minmax(0,2fr)_5rem_6rem_10rem_8rem] items-center gap-3 px-4 py-3 text-xs font-semibold uppercase tracking-[0.18em] md:grid" :class="props.isDark ? 'bg-stone-800/80 text-stone-400' : 'bg-emerald-50/80 text-stone-500'">
        <label class="flex items-center justify-center">
          <input
            ref="selectAllCheckbox"
            class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
            :checked="allSelected"
            type="checkbox"
            @change="emit('toggle-select-all')"
          />
        </label>
        <span>名称</span>
        <span>类型</span>
        <span>大小</span>
        <span>修改时间</span>
        <span class="text-right">操作</span>
      </div>

        <div
          ref="listSurfaceRef"
          class="app-scrollbar relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden"
          :class="dragSelect.active ? 'select-none' : ''"
          @pointerdown="onSurfacePointerDown"
          @pointermove="onSurfacePointerMove"
          @pointerup="onSurfacePointerUp"
          @pointerleave="onSurfacePointerUp"
        >
      <div v-if="busyList" class="px-4 py-12 text-center text-sm" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">正在加载目录...</div>
      <div v-else-if="entries.length === 0" class="px-4 py-12 text-center text-sm" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">当前目录为空。</div>

      <template v-else-if="viewMode === 'list'">
        <article
          v-for="entry in entries"
          :key="entry.fullPath"
          :ref="(el) => setEntryRef(entry.fullPath, el)"
          class="group cursor-pointer select-none border-t px-4 py-3 transition first:border-t-0"
          :class="props.isDark ? 'border-stone-800 hover:bg-stone-800/70' : 'border-stone-200/70 hover:bg-stone-100/60'"
          :title="entryTooltip(entry)"
          @click="handleEntryClick(entry, entries.indexOf(entry), $event)"
          @pointerdown="onEntryPointerDown(entry, $event)"
          @pointermove="onEntryPointerMove"
          @pointerup="cancelLongPress"
          @pointerleave="cancelLongPress"
          @contextmenu.prevent
        >
          <div class="hidden gap-3 md:grid md:grid-cols-[auto_minmax(0,2fr)_5rem_6rem_10rem_8rem] md:items-center">
            <label class="hidden items-center justify-center md:flex" @click.stop>
              <input
                class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
                :checked="isSelected(entry.fullPath)"
                type="checkbox"
                @change="emit('toggle-selection', { path: entry.fullPath, index: entries.indexOf(entry), additiveKey: true })"
              />
            </label>

            <div class="flex min-w-0 items-center gap-3">
              <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl" :class="entry.is_dir ? (props.isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (props.isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
                <IconFolder v-if="entry.is_dir" class="h-5 w-5" />
                <IconFile v-else class="h-5 w-5" />
              </span>
              <span class="min-w-0 truncate text-sm font-medium" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'" :title="entry.name">{{ entry.name }}</span>
            </div>

            <span class="hidden text-sm md:block" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ entry.is_dir ? "目录" : "文件" }}</span>
            <span class="hidden text-sm md:block" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ formatSize(entry.size) }}</span>
            <span class="hidden text-sm md:block" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ formatTime(entry.modified) }}</span>

            <div class="hidden gap-1.5 opacity-0 transition-opacity group-hover:opacity-100 md:flex md:justify-end" @click.stop>
              <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800" title="重命名" @click="emit('rename-entry', entry)">
                <IconPencil class="h-4 w-4" />
              </button>
              <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800 disabled:cursor-not-allowed disabled:opacity-50" :disabled="entry.is_dir" title="下载" @click="emit('download-entry', entry)">
                <IconDownload class="h-4 w-4" />
              </button>
              <button class="flex h-8 w-8 items-center justify-center rounded-full text-rose-500 transition hover:bg-rose-100 hover:text-rose-700" title="删除" @click="emit('delete-entry', entry)">
                <IconTrash class="h-4 w-4" />
              </button>
            </div>
          </div>

          <div class="md:hidden">
            <div class="flex items-start justify-between gap-3">
              <div class="flex min-w-0 items-center gap-3">
                <label v-if="showMobileCheckboxes" class="flex items-center justify-center" @click.stop>
                  <input
                    class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
                    :checked="isSelected(entry.fullPath)"
                    type="checkbox"
                    @change="emit('toggle-selection', { path: entry.fullPath, index: entries.indexOf(entry), additiveKey: true })"
                  />
                </label>
                <span class="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl" :class="entry.is_dir ? (props.isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (props.isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
                  <IconFolder v-if="entry.is_dir" class="h-5 w-5" />
                  <IconFile v-else class="h-5 w-5" />
                </span>
                <span class="min-w-0 truncate text-sm font-medium" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'">{{ entry.name }}</span>
              </div>
              <span class="shrink-0 text-sm" :class="props.isDark ? 'text-stone-300' : 'text-stone-600'">{{ formatSize(entry.size) }}</span>
            </div>
            <div class="mt-2 flex items-center justify-between text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">
              <span>{{ formatTime(entry.modified) }}</span>
              <span>{{ entry.is_dir ? "目录" : "文件" }}</span>
            </div>
          </div>
        </article>
      </template>

      <div v-else class="grid grid-cols-2 gap-3 p-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
        <article
          v-for="entry in entries"
          :key="entry.fullPath"
          :ref="(el) => setEntryRef(entry.fullPath, el)"
          class="group relative cursor-pointer select-none rounded-2xl border p-3 transition hover:-translate-y-0.5 hover:shadow-sm"
          :class="props.isDark ? 'border-stone-700 bg-stone-900/80 hover:border-stone-600' : 'border-stone-200/80 bg-white/70 hover:border-stone-300'"
          :title="entryTooltip(entry)"
          @click="handleEntryClick(entry, entries.indexOf(entry), $event)"
          @pointerdown="onEntryPointerDown(entry, $event)"
          @pointermove="onEntryPointerMove"
          @pointerup="cancelLongPress"
          @pointerleave="cancelLongPress"
          @contextmenu.prevent
        >
          <label class="absolute left-2 top-2 z-10 items-center justify-center rounded-md p-1 shadow-sm" :class="[props.isDark ? 'bg-stone-800/90' : 'bg-white/90', showMobileCheckboxes ? 'flex md:flex' : 'hidden md:flex']" @click.stop>
            <input
              class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
              :checked="isSelected(entry.fullPath)"
              type="checkbox"
              @change="emit('toggle-selection', { path: entry.fullPath, index: entries.indexOf(entry), additiveKey: true })"
            />
          </label>

          <div class="flex h-24 items-center justify-center rounded-xl" :class="entry.is_dir ? (props.isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (props.isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
            <IconFolder v-if="entry.is_dir" class="h-10 w-10" />
            <IconFile v-else class="h-10 w-10" />
          </div>

          <div class="mt-3">
            <p class="truncate text-sm font-semibold" :class="props.isDark ? 'text-stone-100' : 'text-stone-900'" :title="entry.name">{{ entry.name }}</p>
            <p class="mt-1 text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ entry.is_dir ? "目录" : formatSize(entry.size) }}</p>
            <p class="mt-1 text-xs" :class="props.isDark ? 'text-stone-400' : 'text-stone-500'">{{ formatTime(entry.modified) }}</p>
          </div>

          <div class="mt-3 hidden gap-1.5 opacity-0 transition-opacity group-hover:opacity-100 md:flex" @click.stop>
            <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800" title="重命名" @click="emit('rename-entry', entry)">
              <IconPencil class="h-4 w-4" />
            </button>
            <button class="flex h-8 w-8 items-center justify-center rounded-full text-stone-500 transition hover:bg-stone-200 hover:text-stone-800 disabled:cursor-not-allowed disabled:opacity-50" :disabled="entry.is_dir" title="下载" @click="emit('download-entry', entry)">
              <IconDownload class="h-4 w-4" />
            </button>
            <button class="flex h-8 w-8 items-center justify-center rounded-full text-rose-500 transition hover:bg-rose-100 hover:text-rose-700" title="删除" @click="emit('delete-entry', entry)">
              <IconTrash class="h-4 w-4" />
            </button>
          </div>
        </article>
      </div>

      <div
        v-if="dragSelect.active && dragSelect.moved"
        class="pointer-events-none absolute z-20 rounded-md border border-sky-500/70 bg-sky-400/15"
        :style="{ left: `${dragSelect.x}px`, top: `${dragSelect.y}px`, width: `${dragSelect.w}px`, height: `${dragSelect.h}px` }"
      ></div>
        </div>
      </div>
    </div>

    <div v-if="actionMenuEntry" class="fixed inset-0 z-40 bg-black/35 md:hidden" @click="closeActionMenu()">
      <div class="absolute left-1/2 top-1/2 w-[min(22rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-[1.75rem] p-4 shadow-2xl" :class="props.isDark ? 'bg-stone-900 text-stone-100' : 'bg-white text-stone-900'" @click.stop>
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
          <button class="rounded-2xl px-4 py-3 text-left text-sm font-medium" :class="props.isDark ? 'bg-stone-800' : 'bg-stone-100'" @click="openMobileAction('select')">多选</button>
          <button class="rounded-2xl px-4 py-3 text-left text-sm font-medium" :class="props.isDark ? 'bg-stone-800' : 'bg-stone-100'" @click="openMobileAction('rename')">重命名</button>
          <button v-if="!actionMenuEntry.is_dir" class="rounded-2xl px-4 py-3 text-left text-sm font-medium" :class="props.isDark ? 'bg-stone-800' : 'bg-stone-100'" @click="openMobileAction('download')">下载</button>
          <button class="rounded-2xl bg-rose-600 px-4 py-3 text-left text-sm font-medium text-white" @click="openMobileAction('delete')">删除</button>
        </div>
      </div>
    </div>

    <UploadModal
      v-if="showUploadModal"
      :busy-upload="busyUpload"
      @close="showUploadModal = false"
      @upload-directory="(files) => { emit('upload-directory', files); showUploadModal = false; }"
      @upload-files="(files) => { emit('upload-files', files); showUploadModal = false; }"
    />
  </section>
</template>