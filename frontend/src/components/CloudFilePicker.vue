<script setup>
import { computed, ref, watch } from "vue";
import { prettyPath } from "../utils/path";
import { isImageEntry, thumbnailUrl } from "../utils/fileBrowser";
import { t } from "../i18n/index.js";
import AppDialog from "./AppDialog.vue";
import {
  IconArrowLeft,
  IconChevronRight,
  IconClose,
  IconFile,
  IconFolder,
  IconGrid,
  IconHome,
  IconList,
} from "../utils/icons";

const props = defineProps({
  show: { type: Boolean, default: true },
  isDark: { type: Boolean, default: false },
  fetchList: { type: Function, required: true },
});

const emit = defineEmits(["confirm", "close"]);

const currentPath = ref("");
const entries = ref([]);
const loading = ref(false);
const error = ref("");
const selected = ref(new Set());
const viewMode = ref("grid");
const selectAllCheckbox = ref(null);

const breadcrumbs = computed(() => {
  if (!currentPath.value) {
    return [];
  }

  const parts = currentPath.value.split("/").filter(Boolean);
  return parts.map((name, index) => ({
    name,
    path: parts.slice(0, index + 1).join("/"),
  }));
});

const currentPathLabel = computed(() => prettyPath(currentPath.value));
const currentSelectionCount = computed(() =>
  entries.value.filter((entry) => selected.value.has(entry.fullPath)).length,
);
const allSelected = computed(() => entries.value.length > 0 && currentSelectionCount.value === entries.value.length);
const selectAllIndeterminate = computed(() => currentSelectionCount.value > 0 && !allSelected.value);

watch(selectAllIndeterminate, (value) => {
  if (selectAllCheckbox.value) {
    selectAllCheckbox.value.indeterminate = value;
  }
}, { immediate: true });

async function loadPath(path) {
  loading.value = true;
  error.value = "";
  entries.value = [];

  try {
    const result = await props.fetchList(path);
    entries.value = (result.items || []).map((entry) => ({
      ...entry,
      fullPath: path ? `${path}/${entry.name}` : entry.name,
    }));
  } catch (err) {
    error.value = err.message || String(err);
  } finally {
    loading.value = false;
  }
}

watch(currentPath, (path) => {
  if (props.show) loadPath(path);
}, { immediate: true });

watch(() => props.show, (open) => {
  if (open) loadPath(currentPath.value);
});

function isSelected(path) {
  return selected.value.has(path);
}

function toggleEntry(entry) {
  const next = new Set(selected.value);
  if (next.has(entry.fullPath)) {
    next.delete(entry.fullPath);
  } else {
    next.add(entry.fullPath);
  }
  selected.value = next;
}

function toggleSelectAll() {
  const next = new Set(selected.value);
  if (allSelected.value) {
    entries.value.forEach((entry) => next.delete(entry.fullPath));
  } else {
    entries.value.forEach((entry) => next.add(entry.fullPath));
  }
  selected.value = next;
}

function openEntry(entry) {
  if (entry.is_dir) {
    navigateTo(entry.fullPath);
    return;
  }
  toggleEntry(entry);
}

function navigateTo(path) {
  selected.value = new Set();
  currentPath.value = path;
}

function goUp() {
  if (!currentPath.value) {
    return;
  }

  selected.value = new Set();
  const parts = currentPath.value.split("/").filter(Boolean);
  parts.pop();
  currentPath.value = parts.join("/");
}

function confirm() {
  emit("confirm", Array.from(selected.value));
}
</script>

<template>
  <AppDialog :show="show" :z-index="130" @close="emit('close')">
      <div
        class="app-dialog-panel app-dialog-panel-soft flex h-[min(86vh,860px)] w-full max-w-5xl flex-col overflow-hidden rounded-[2rem] border"
        :class="isDark ? 'text-stone-100' : 'text-stone-900'"
      >
        <div class="flex shrink-0 items-center justify-between border-b px-5 py-4 md:px-6"
          :class="isDark ? 'border-stone-700' : 'border-stone-200'">
          <div class="min-w-0">
            <p class="text-xs font-semibold uppercase tracking-[0.24em]"
              :class="isDark ? 'text-teal-300' : 'text-teal-700'">
              {{ t('import.cloud') }}
            </p>
            <h3 class="mt-1 truncate text-base font-semibold md:text-lg">{{ t('import.cloudPickerTitle') }}</h3>
            <p class="mt-1 truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
              {{ currentPathLabel }}
            </p>
          </div>
          <button
            class="flex h-9 w-9 items-center justify-center rounded-full transition"
            :class="isDark ? 'text-stone-400 hover:bg-stone-800 hover:text-stone-200' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-700'"
            @click="emit('close')"
          >
            <IconClose class="h-4 w-4" />
          </button>
        </div>

        <div class="flex shrink-0 flex-wrap items-center gap-2 border-b px-5 py-3 md:px-6"
          :class="isDark ? 'border-stone-700' : 'border-stone-200'">
          <button
            class="inline-flex items-center gap-1.5 rounded-xl px-3 py-2 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-50"
            :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-700 hover:bg-stone-200'"
            :disabled="!currentPath"
            @click="goUp"
          >
            <IconArrowLeft class="h-4 w-4" />
            {{ t('files.goUp') }}
          </button>
          <div class="min-w-0 flex-1 overflow-x-auto">
            <div class="flex min-w-max items-center gap-1 text-sm">
              <button
                class="inline-flex items-center gap-1 rounded-full px-3 py-1.5 font-medium transition"
                :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-100 text-stone-800 hover:bg-stone-200'"
                @click="navigateTo('')"
              >
                <IconHome class="h-4 w-4" />
                {{ t('files.root') }}
              </button>
              <template v-for="crumb in breadcrumbs" :key="crumb.path">
                <IconChevronRight class="h-4 w-4 shrink-0" :class="isDark ? 'text-stone-500' : 'text-stone-400'" />
                <button
                  class="truncate rounded-full px-3 py-1.5 font-medium transition"
                  :class="crumb.path === currentPath
                    ? (isDark ? 'bg-teal-500/20 text-teal-200' : 'bg-teal-100 text-teal-800')
                    : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-700 hover:bg-stone-200')"
                  @click="navigateTo(crumb.path)"
                >
                  {{ crumb.name }}
                </button>
              </template>
            </div>
          </div>
          <div class="inline-flex items-center gap-1 rounded-xl p-1" :class="isDark ? 'bg-stone-800/90' : 'bg-stone-100/90'">
            <button
              class="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition"
              :class="viewMode === 'list'
                ? (isDark ? 'bg-stone-100 text-stone-900' : 'bg-white text-stone-900 shadow-sm')
                : (isDark ? 'text-stone-200 hover:bg-stone-700' : 'text-stone-700 hover:bg-stone-200')"
              @click="viewMode = 'list'"
            >
              <IconList class="h-4 w-4" />
              <span class="hidden sm:inline">{{ t('files.listView') }}</span>
            </button>
            <button
              class="inline-flex items-center gap-1.5 rounded-lg px-3 py-2 text-sm font-medium transition"
              :class="viewMode === 'grid'
                ? (isDark ? 'bg-stone-100 text-stone-900' : 'bg-white text-stone-900 shadow-sm')
                : (isDark ? 'text-stone-200 hover:bg-stone-700' : 'text-stone-700 hover:bg-stone-200')"
              @click="viewMode = 'grid'"
            >
              <IconGrid class="h-4 w-4" />
              <span class="hidden sm:inline">{{ t('files.gridView') }}</span>
            </button>
          </div>
        </div>

        <div class="flex shrink-0 items-center justify-between gap-3 border-b px-5 py-3 md:px-6"
          :class="isDark ? 'border-stone-700' : 'border-stone-200'">
          <label class="inline-flex items-center gap-2 text-sm font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
            <input
              ref="selectAllCheckbox"
              class="h-4 w-4 rounded border-stone-300 text-teal-600 focus:ring-teal-200"
              :checked="allSelected"
              type="checkbox"
              @change="toggleSelectAll"
            />
            {{ allSelected ? t('files.deselectAll') : t('files.selectAll') }}
          </label>
          <span class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ selected.size > 0 ? t('picker.selectedCount', { count: selected.size }) : t('picker.noneSelected') }}
          </span>
        </div>

        <div class="app-scrollbar min-h-0 flex-1 overflow-y-auto px-4 py-4 md:px-5">
          <div v-if="loading" class="py-12 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ t('files.loading') }}
          </div>
          <div v-else-if="error" class="py-12 text-center text-sm text-rose-500">{{ error }}</div>
          <div v-else-if="entries.length === 0" class="py-12 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ t('files.empty') }}
          </div>

          <template v-else-if="viewMode === 'list'">
            <div class="overflow-hidden rounded-[1.5rem] border" :class="isDark ? 'border-stone-700 bg-stone-900/60' : 'border-stone-200 bg-stone-50/70'">
              <div class="hidden grid-cols-[auto_minmax(0,2fr)_5rem_6rem] items-center gap-3 px-4 py-3 text-xs font-semibold uppercase tracking-[0.18em] md:grid"
                :class="isDark ? 'bg-stone-800/80 text-stone-400' : 'bg-teal-50 text-stone-500'">
                <span></span>
                <span>{{ t('files.name') }}</span>
                <span>{{ t('files.type') }}</span>
                <span>{{ t('files.size') }}</span>
              </div>
              <article
                v-for="entry in entries"
                :key="entry.fullPath"
                class="group cursor-pointer border-t px-4 py-3 transition first:border-t-0"
                :class="[
                  isSelected(entry.fullPath)
                    ? (isDark ? 'border-stone-700 bg-teal-500/10' : 'border-stone-200 bg-teal-50')
                    : (isDark ? 'border-stone-800 hover:bg-stone-800/70' : 'border-stone-200 hover:bg-stone-100/70'),
                ]"
                @click="openEntry(entry)"
              >
                <div class="hidden items-center gap-3 md:grid md:grid-cols-[auto_minmax(0,2fr)_5rem_6rem]">
                  <label class="flex items-center justify-center" @click.stop>
                    <input
                      class="h-4 w-4 rounded border-stone-300 text-teal-600 focus:ring-teal-200"
                      :checked="isSelected(entry.fullPath)"
                      type="checkbox"
                      @change="toggleEntry(entry)"
                    />
                  </label>
                  <div class="flex min-w-0 items-center gap-3">
                    <span class="relative flex h-10 w-10 shrink-0 items-center justify-center overflow-hidden rounded-xl"
                      :class="entry.is_dir ? (isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
                      <img
                        v-if="isImageEntry(entry)"
                        class="absolute inset-0 h-full w-full object-cover"
                        :src="thumbnailUrl(entry, 96)"
                        loading="lazy"
                        decoding="async"
                        fetchpriority="low"
                        alt=""
                      />
                      <IconFolder v-if="entry.is_dir" class="h-5 w-5" />
                      <IconFile v-else-if="!isImageEntry(entry)" class="h-5 w-5" />
                    </span>
                    <div class="min-w-0">
                      <p class="truncate text-sm font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ entry.name }}</p>
                      <p class="truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ prettyPath(entry.fullPath) }}</p>
                    </div>
                  </div>
                  <span class="text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ entry.is_dir ? t('files.directory') : t('files.file') }}</span>
                  <span class="text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ entry.is_dir ? '--' : `${Math.max(1, Math.round((entry.size || 0) / 1024))} KB` }}</span>
                </div>

                <div class="md:hidden">
                  <div class="flex items-start gap-3">
                    <label class="mt-2 flex items-center justify-center" @click.stop>
                      <input
                        class="h-4 w-4 rounded border-stone-300 text-teal-600 focus:ring-teal-200"
                        :checked="isSelected(entry.fullPath)"
                        type="checkbox"
                        @change="toggleEntry(entry)"
                      />
                    </label>
                    <span class="relative flex h-12 w-12 shrink-0 items-center justify-center overflow-hidden rounded-xl"
                      :class="entry.is_dir ? (isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
                      <img
                        v-if="isImageEntry(entry)"
                        class="absolute inset-0 h-full w-full object-cover"
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
                      <p class="truncate text-sm font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ entry.name }}</p>
                      <p class="mt-1 truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ prettyPath(entry.fullPath) }}</p>
                      <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ entry.is_dir ? t('files.directory') : t('files.file') }}</p>
                    </div>
                  </div>
                </div>
              </article>
            </div>
          </template>

          <div v-else class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
            <article
              v-for="entry in entries"
              :key="entry.fullPath"
              class="group relative cursor-pointer rounded-2xl border p-3 transition hover:-translate-y-0.5 hover:shadow-sm"
              :class="[
                isSelected(entry.fullPath)
                  ? (isDark ? 'border-teal-500 bg-teal-500/10' : 'border-teal-300 bg-teal-50')
                  : (isDark ? 'border-stone-700 bg-stone-900/80 hover:border-stone-600' : 'border-stone-200 bg-white/70 hover:border-stone-300'),
              ]"
              @click="openEntry(entry)"
            >
              <label class="absolute left-2 top-2 z-10 flex items-center justify-center rounded-md p-1 shadow-sm"
                :class="isDark ? 'bg-stone-800/90' : 'bg-white/90'"
                @click.stop>
                <input
                  class="h-4 w-4 rounded border-stone-300 text-teal-600 focus:ring-teal-200"
                  :checked="isSelected(entry.fullPath)"
                  type="checkbox"
                  @change="toggleEntry(entry)"
                />
              </label>

              <div class="relative flex h-28 items-center justify-center overflow-hidden rounded-xl"
                :class="entry.is_dir ? (isDark ? 'bg-amber-900/40 text-amber-300' : 'bg-amber-100 text-amber-700') : (isDark ? 'bg-stone-800 text-stone-300' : 'bg-stone-200 text-stone-600')">
                <img
                  v-if="isImageEntry(entry)"
                  class="absolute inset-0 h-full w-full object-cover"
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
                <p class="truncate text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ entry.name }}</p>
                <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ entry.is_dir ? t('files.directory') : t('files.file') }}</p>
                <p class="mt-1 truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ prettyPath(entry.fullPath) }}</p>
              </div>
            </article>
          </div>
        </div>

        <div class="flex shrink-0 items-center justify-between gap-3 border-t px-5 py-4 md:px-6"
          :class="isDark ? 'border-stone-700' : 'border-stone-200'">
          <span class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ selected.size > 0 ? t('picker.selectedCount', { count: selected.size }) : t('picker.noneSelected') }}
          </span>
          <div class="flex gap-2">
            <button
              class="rounded-xl px-4 py-2 text-sm transition"
              :class="isDark ? 'bg-stone-800 text-stone-300 hover:bg-stone-700' : 'bg-stone-100 text-stone-700 hover:bg-stone-200'"
              @click="emit('close')"
            >{{ t('btn.cancel') }}</button>
            <button
              class="rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="selected.size > 0
                ? 'bg-teal-500 text-white hover:bg-teal-600'
                : (isDark ? 'bg-stone-700 text-stone-500 cursor-not-allowed' : 'bg-stone-200 text-stone-400 cursor-not-allowed')"
              :disabled="selected.size === 0"
              @click="confirm"
            >{{ t('btn.confirm') }}</button>
          </div>
        </div>
      </div>
  </AppDialog>
</template>
