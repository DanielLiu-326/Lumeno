<script setup>
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import AlbumPanel from "./components/AlbumPanel.vue";
import AppSidebar from "./components/AppSidebar.vue";
import ConflictModal from "./components/ConflictModal.vue";
import DailyPicksPanel from "./components/DailyPicksPanel.vue";
import FileBrowserPanel from "./components/FileBrowserPanel.vue";
import LoginPanel from "./components/LoginPanel.vue";
import OnThisDayPanel from "./components/OnThisDayPanel.vue";
import SettingsPanel from "./components/SettingsPanel.vue";
import { useAlbumApp } from "./composables/useAlbumApp";
import { progressPercent, statusLabel, statusTone } from "./utils/format";
import { IconBell, IconClose, IconMenu, IconMoon, IconPower, IconSun, IconTasks } from "./utils/icons";
import { t, locale } from "./i18n/index.js";

const app = proxyRefs(useAlbumApp());
const activeTab = ref("files");
const mobileSidebarOpen = ref(false);
const taskListOpen = ref(false);
const notificationOpen = ref(false);
const toastVisible = ref(false);
const toastMessage = ref("");
const theme = ref("light");
const isMobile = ref(false);
let toastTimer = null;

const notifications = ref([]);

const activeTasks = computed(() => app.tasks.filter((task) => [0, 1, 2].includes(task.status)));
const activeTaskCount = computed(() => activeTasks.value.length);
const averageTaskProgress = computed(() => {
  if (activeTasks.value.length === 0) {
    return 0;
  }
  const total = activeTasks.value.reduce((acc, task) => acc + progressPercent(task), 0);
  return Math.round(total / activeTasks.value.length);
});
const taskPieStyle = computed(() => ({
  background: `conic-gradient(#f97316 ${averageTaskProgress.value}%, #e7e5e4 0)`,
}));
const unreadNotificationCount = computed(
  () => notifications.value.filter((item) => !item.read).length,
);

const isDark = computed(() => theme.value === "dark");
const activeTabTitle = computed(() => {
  if (activeTab.value === "albums") return t("nav.albums");
  if (activeTab.value === "settings") return t("nav.settings");
  if (activeTab.value === "on_this_day") return t("nav.on_this_day");
  if (activeTab.value === "daily_picks") return t("nav.daily_picks");
  return t("nav.files");
});

function applyTheme(nextTheme) {
  theme.value = nextTheme;
  document.documentElement.classList.toggle("dark", nextTheme === "dark");
  localStorage.setItem("album-theme", nextTheme);
}

function updateViewportMode() {
  isMobile.value = window.innerWidth < 768;
}

function toggleTheme() {
  applyTheme(isDark.value ? "light" : "dark");
}

function openTaskList() {
  taskListOpen.value = !taskListOpen.value;
  if (taskListOpen.value) {
    notificationOpen.value = false;
  }
}

function openNotifications() {
  notificationOpen.value = !notificationOpen.value;
  if (notificationOpen.value) {
    taskListOpen.value = false;
  }
  if (notificationOpen.value) {
    notifications.value = notifications.value.map((item) => ({ ...item, read: true }));
  }
}

function clearNotifications() {
  notifications.value = [];
}

async function clearFinishedTasksFromPanel() {
  await app.clearFinishedTasks();
}

onMounted(() => {
  const saved = localStorage.getItem("album-theme");
  applyTheme(saved === "dark" ? "dark" : "light");
  updateViewportMode();
  document.addEventListener("click", onGlobalClick);
  document.addEventListener("keydown", onGlobalKeyDown);
  window.addEventListener("resize", updateViewportMode);
});

onBeforeUnmount(() => {
  document.removeEventListener("click", onGlobalClick);
  document.removeEventListener("keydown", onGlobalKeyDown);
  window.removeEventListener("resize", updateViewportMode);
  if (toastTimer) {
    clearTimeout(toastTimer);
  }
});

function onGlobalClick(event) {
  const target = event.target;
  if (!(target instanceof HTMLElement)) {
    return;
  }
  if (!target.closest("[data-topbar-menu]")) {
    taskListOpen.value = false;
    notificationOpen.value = false;
  }
}

function onGlobalKeyDown(event) {
  const target = event.target;
  if (target instanceof HTMLElement && target.closest("input, textarea, [contenteditable='true']")) {
    return;
  }
  if (event.key === "Escape") {
    mobileSidebarOpen.value = false;
    taskListOpen.value = false;
    notificationOpen.value = false;
    return;
  }
  if (!(event.ctrlKey || event.metaKey)) {
    return;
  }
  const key = event.key.toLowerCase();
  if (key === "a") {
    event.preventDefault();
    app.selectAllEntries();
  } else if (key === "c") {
    if (app.selectedEntries.length > 0) {
      event.preventDefault();
      app.copySelected();
    }
  } else if (key === "x") {
    if (app.selectedEntries.length > 0) {
      event.preventDefault();
      app.cutSelected();
    }
  }
}

watch(
  () => app.message.sequence,
  () => {
    const text = app.message.text;
    if (!text) {
      return;
    }
    const item = {
      id: Date.now() + Math.random(),
      text,
      kind: app.message.kind,
      createdAt: new Date().toLocaleTimeString(locale.value === "zh" ? "zh-CN" : "en-US", { hour12: false }),
      read: false,
    };
    notifications.value = [item, ...notifications.value].slice(0, 40);
    toastMessage.value = text;
    toastVisible.value = true;
    if (toastTimer) {
      clearTimeout(toastTimer);
    }
    toastTimer = setTimeout(() => {
      toastVisible.value = false;
    }, 3200);
  },
  { immediate: false },
);

watch(
  () => activeTab.value,
  async (tab) => {
    if (tab === "albums") {
      await app.refreshAlbums();
    }
  },
);
</script>

<template>
  <!-- Login: full-screen standalone page -->
  <LoginPanel
    v-if="!app.loggedIn"
    :busy-login="app.busy.login"
    :login-form="app.loginForm"
    @submit="app.login"
    @register="app.register"
  />

  <!-- Main layout: sidebar + content -->
  <div v-else class="flex h-screen overflow-hidden" :class="isDark ? 'bg-stone-950 text-stone-100' : ''">
    <AppSidebar
      class="hidden md:flex"
      :active-tab="activeTab"
      :is-dark="isDark"
      :username="app.currentUser"
      :avatar="app.userAvatar"
      @switch-tab="(tab) => (activeTab = tab)"
    />

    <Transition
      enter-active-class="transition-opacity duration-200 ease-out"
      leave-active-class="transition-opacity duration-200 ease-in"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div
        v-if="mobileSidebarOpen"
        class="fixed inset-0 z-[70] bg-black/35 md:hidden"
        @click="mobileSidebarOpen = false"
      ></div>
    </Transition>
    <Transition
      enter-active-class="transition-transform duration-[220ms] ease-out"
      leave-active-class="transition-transform duration-200 ease-in"
      enter-from-class="-translate-x-full"
      leave-to-class="-translate-x-full"
    >
      <div v-if="mobileSidebarOpen" class="fixed inset-y-0 left-0 z-[71] w-72 md:hidden">
        <AppSidebar
          class="flex h-full w-full"
          :active-tab="activeTab"
          :is-dark="isDark"
          :username="app.currentUser"
          :avatar="app.userAvatar"
          @switch-tab="(tab) => { activeTab = tab; mobileSidebarOpen = false; }"
        />
      </div>
    </Transition>

    <div class="relative flex flex-1 flex-col overflow-hidden">
      <!-- Background -->
      <div
        class="pointer-events-none absolute inset-0"
        :style="isDark ? 'background: radial-gradient(circle at top left, rgba(251, 146, 60, 0.12), transparent 32%), radial-gradient(circle at top right, rgba(20, 184, 166, 0.12), transparent 26%), linear-gradient(160deg, #111827 0%, #0f172a 44%, #020617 100%)' : 'background: radial-gradient(circle at top left, rgba(177, 77, 40, 0.18), transparent 28%), radial-gradient(circle at top right, rgba(29, 106, 97, 0.18), transparent 22%), linear-gradient(160deg, #f8f3ec 0%, #f4efe7 44%, #efe4d6 100%)'"
      ></div>
      <div
        class="pointer-events-none absolute inset-0 opacity-25 [background-image:linear-gradient(rgba(125,98,73,0.06)_1px,transparent_1px),linear-gradient(90deg,rgba(125,98,73,0.06)_1px,transparent_1px)] [background-size:28px_28px] [mask-image:linear-gradient(to_bottom,rgba(0,0,0,0.75),transparent_88%)]"
      ></div>

      <div class="relative z-50 border-b px-4 py-3 backdrop-blur" :class="isDark ? 'border-stone-700/70 bg-stone-900/85' : 'border-stone-200/60 bg-white/55'">
        <div class="flex items-center justify-between gap-3">
          <div class="flex min-w-0 items-center gap-3">
            <button
              class="inline-flex h-10 w-10 items-center justify-center rounded-xl transition md:hidden"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              :title="t('topbar.openSidebar')"
              @click="mobileSidebarOpen = true"
            >
              <IconMenu class="h-5 w-5" />
            </button>
            <div class="min-w-0">
              <p class="text-xs uppercase tracking-[0.2em] text-stone-500">{{ activeTabTitle }}</p>
            </div>
          </div>

          <div class="relative flex items-center gap-2" data-topbar-menu>
            <button
              class="relative inline-flex h-10 w-10 items-center justify-center rounded-xl transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              :title="t('topbar.tasks')"
              @click.stop="openTaskList"
            >
              <template v-if="activeTaskCount > 0">
                <span class="relative flex h-6 w-6 items-center justify-center rounded-full" :style="taskPieStyle">
                  <span class="h-3.5 w-3.5 rounded-full" :class="isDark ? 'bg-stone-900' : 'bg-[#f6efe6]'"></span>
                </span>
              </template>
              <IconTasks v-else class="h-5 w-5" />
              <span
                v-if="activeTaskCount > 0"
                class="absolute -right-1 -top-1 inline-flex min-w-5 items-center justify-center rounded-full bg-rose-600 px-1.5 py-0.5 text-[11px] font-semibold text-white"
              >
                {{ activeTaskCount }}
              </span>
            </button>

            <button
              class="relative inline-flex h-10 w-10 items-center justify-center rounded-xl transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              :title="t('topbar.notifications')"
              @click.stop="openNotifications"
            >
              <IconBell class="h-5 w-5" />
              <span
                v-if="unreadNotificationCount > 0"
                class="absolute -right-1 -top-1 inline-flex min-w-5 items-center justify-center rounded-full bg-rose-600 px-1.5 py-0.5 text-[11px] font-semibold text-white"
              >
                {{ unreadNotificationCount }}
              </span>
            </button>

            <button
              class="inline-flex h-10 w-10 items-center justify-center rounded-xl transition"
              :class="isDark ? 'bg-stone-800 text-amber-300 hover:bg-stone-700' : 'bg-stone-200 text-amber-700 hover:bg-stone-300'"
              :title="isDark ? t('topbar.switchToLight') : t('topbar.switchToDark')"
              @click="toggleTheme"
            >
              <IconSun v-if="isDark" class="h-5 w-5" />
              <IconMoon v-else class="h-5 w-5" />
            </button>

            <button
              class="inline-flex h-10 w-10 items-center justify-center rounded-xl bg-rose-600 text-white transition hover:bg-rose-700"
              @click="app.logout"
              :title="t('topbar.logout')"
            >
              <IconPower class="h-4 w-4" />
            </button>

            <div
              v-if="notificationOpen"
              class="z-40 overflow-hidden border shadow-xl md:absolute md:right-0 md:top-12 md:w-[22rem] md:rounded-2xl"
              :class="isMobile ? (isDark ? 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-stone-950' : 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-white') : (isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white')"
            >
              <div class="flex items-center justify-between border-b px-4 py-3" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
                <p class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t('topbar.notificationCenter') }}</p>
                <div class="flex items-center gap-2">
                  <button class="inline-flex items-center rounded-full px-3 py-1.5 text-xs font-medium transition" :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="clearNotifications">
                    {{ t('topbar.clear') }}
                  </button>
                  <button class="inline-flex h-9 w-9 items-center justify-center rounded-full transition" :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="notificationOpen = false">
                    <IconClose class="h-4 w-4" />
                  </button>
                </div>
              </div>
              <div class="app-scrollbar overflow-y-auto overflow-x-hidden" :class="isMobile ? 'h-[calc(100vh-4rem)]' : 'max-h-80'">
                <div
                  v-for="item in notifications"
                  :key="item.id"
                  class="border-b px-4 py-3 last:border-b-0"
                  :class="isDark ? 'border-stone-800' : 'border-stone-100'"
                >
                  <p class="truncate text-sm" :class="item.kind === 'error' ? 'text-rose-500' : (isDark ? 'text-stone-200' : 'text-stone-800')" :title="item.text">
                    {{ item.text }}
                  </p>
                  <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ item.createdAt }}</p>
                </div>
                <div v-if="notifications.length === 0" class="px-4 py-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  {{ t('topbar.noNotifications') }}
                </div>
              </div>
            </div>

            <div
              v-if="taskListOpen"
              class="z-40 overflow-hidden border shadow-xl md:absolute md:right-12 md:top-12 md:w-[26rem] md:rounded-2xl"
              :class="isMobile ? (isDark ? 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-stone-950' : 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-white') : (isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white')"
            >
              <div class="flex items-center justify-between border-b px-4 py-3" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
                <p class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t('topbar.tasks') }}</p>
                <div class="flex items-center gap-2">
                  <button class="inline-flex items-center rounded-full px-3 py-1.5 text-xs font-medium transition" :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="clearFinishedTasksFromPanel">
                    {{ t('topbar.clearFinished') }}
                  </button>
                  <button class="inline-flex h-9 w-9 items-center justify-center rounded-full transition" :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="taskListOpen = false">
                    <IconClose class="h-4 w-4" />
                  </button>
                </div>
              </div>
              <div class="app-scrollbar overflow-y-auto overflow-x-hidden" :class="isMobile ? 'h-[calc(100vh-4rem)]' : 'max-h-96'">
                <div
                  v-for="task in app.tasks"
                  :key="task.id"
                  class="border-b px-4 py-3 last:border-b-0"
                  :class="isDark ? 'border-stone-800' : 'border-stone-100'"
                >
                  <div class="flex min-w-0 items-start justify-between gap-3">
                    <p class="min-w-0 truncate text-sm font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-900'" :title="task.name">{{ task.name }}</p>
                    <div class="flex shrink-0 items-center gap-2">
                      <button
                        v-if="task.cancellable && (task.status === 0 || task.status === 1)"
                        class="rounded-full px-2.5 py-1 text-[11px] font-medium transition"
                        :class="isDark ? 'bg-rose-900/50 text-rose-300 hover:bg-rose-800/60' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
                        @click="app.cancelTask(task.id)"
                      >{{ t('tasks.cancel') }}</button>
                      <span class="shrink-0 rounded-full px-2.5 py-1 text-[11px] font-medium" :class="statusTone(task.status)">{{ statusLabel(task.status) }}</span>
                    </div>
                  </div>
                  <p class="mt-1 truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'" :title="task.current_task">{{ task.current_task || t('topbar.waitingForUpdate') }}</p>
                  <div class="mt-2 h-1.5 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-700' : 'bg-stone-200'">
                    <div
                      class="h-full rounded-full bg-linear-to-r from-amber-600 via-orange-400 to-emerald-600"
                      :style="{ width: `${progressPercent(task)}%` }"
                    ></div>
                  </div>
                </div>
                <div v-if="app.tasks.length === 0" class="px-4 py-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  {{ t('topbar.noTasks') }}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="relative z-10 flex flex-1 gap-5 overflow-hidden p-3 md:p-6">
        <template v-if="activeTab === 'files'">
          <div class="min-h-0 min-w-0 w-full">
            <FileBrowserPanel
              :is-dark="isDark"
              :all-selected="app.allSelected"
              :breadcrumb-parts="app.breadcrumbParts"
              :busy-list="app.busy.list"
              :busy-upload="app.busy.upload"
              :can-go-up="app.canGoUp"
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
            />
          </div>
        </template>

        <template v-else-if="activeTab === 'albums'">
          <div class="min-h-0 min-w-0 w-full">
            <AlbumPanel
              :is-dark="isDark"
              :busy-albums="app.busy.albums"
              :albums="app.albums"
              :shared-albums="app.sharedAlbums"
              :current-album="app.currentAlbum"
              :current-user="app.currentUser"
              :album-photos="app.albumPhotos"
              :available-tags="app.albumAvailableTags"
              :available-cameras="app.albumAvailableCameras"
              :available-lenses="app.albumAvailableLenses"
              :available-formats="app.albumAvailableFormats"
              :date-histogram="app.dateHistogram"
              :filters="app.albumFilters"
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
              @create-album="app.createAlbum"
              @refresh-albums="app.refreshAlbums"
              @select-album="app.selectAlbum"
              @update-album="app.updateAlbum"
              @delete-album="app.deleteAlbum"
              @delete-photos="app.deleteAlbumPhotos"
              @share-album="app.shareAlbum"
              @unshare-album="app.unshareAlbum"
              @apply-filters="app.refreshAlbumPhotos"
              @update-sort="(s) => { app.albumSort = s; app.refreshAlbumPhotos(); }"
              @import-selected="app.importSelectedToAlbum"
              @import-local="(files) => app.importLocalToAlbum(files)"
              @import-cloud="(paths) => app.importCloudToAlbum(paths)"
              @update-photo-meta="app.updatePhotoMeta"
              @copy-photo-to-album="app.copyPhotoToAlbum"
              @rebuild-index="app.rebuildAlbumIndex"
              @load-more-photos="app.loadMoreAlbumPhotos"
              @select-collection="app.selectCollection"
            />
          </div>
        </template>

        <template v-else-if="activeTab === 'settings'">
          <div class="min-h-0 min-w-0 w-full">
            <SettingsPanel
              :is-dark="isDark"
              :members="app.members"
              :settings="app.settings"
              :is-admin="app.isAdmin"
              :current-user="app.currentUser"
              :current-locale="app.userLocale"
              :user-avatar="app.userAvatar"
              :on-update-password="app.updatePassword"
              :on-upload-avatar="app.uploadAvatar"
              :user-role="app.userRole"
              :webdav-shares="app.webdavShares"
              :on-webdav-update="app.updateWebdavShare"
              @update-settings="app.updateSettings"
              @update-member="app.updateMember"
              @update-password="app.updatePassword"
              @update-preferences="app.updatePreferences"
              @delete-member="app.deleteMember"
              @reset-member-password="({ username, new_password }) => app.resetMemberPassword(username, new_password)"
            />
          </div>
        </template>

        <template v-else-if="activeTab === 'on_this_day'">
          <div class="min-h-0 min-w-0 w-full h-full">
            <OnThisDayPanel :is-dark="isDark" />
          </div>
        </template>

        <template v-else-if="activeTab === 'daily_picks'">
          <div class="min-h-0 min-w-0 w-full h-full">
            <DailyPicksPanel :is-dark="isDark" />
          </div>
        </template>
      </div>

      <!-- Conflict resolution modal -->
      <ConflictModal
        v-if="app.conflictModalVisible"
        :is-dark="isDark"
        @choose="app.resolveConflict"
        @cancel="app.cancelConflict"
      />

      <teleport to="body">
        <transition
          enter-active-class="transition duration-200"
          leave-active-class="transition duration-200"
          enter-from-class="-translate-y-2 opacity-0"
          leave-to-class="-translate-y-2 opacity-0"
        >
          <button
            v-if="toastVisible"
            class="fixed left-1/2 top-4 z-[140] w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2 rounded-xl border px-4 py-3 text-left shadow-2xl"
            :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'"
            @click="notificationOpen = true; toastVisible = false"
            :title="toastMessage"
          >
            <p class="truncate text-sm" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ toastMessage }}</p>
            <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('topbar.clickForNotifications') }}</p>
          </button>
        </transition>
      </teleport>
    </div>
  </div>

  <!-- Toast visible on all pages (including login) -->
  <teleport to="body">
    <transition
      enter-active-class="transition duration-200"
      leave-active-class="transition duration-200"
      enter-from-class="-translate-y-2 opacity-0"
      leave-to-class="-translate-y-2 opacity-0"
    >
      <div
        v-if="toastVisible && !app.loggedIn"
        class="fixed left-1/2 top-4 z-[140] w-[min(32rem,calc(100vw-2rem))] -translate-x-1/2 rounded-xl border border-stone-200 bg-white px-4 py-3 text-left shadow-2xl"
        :title="toastMessage"
      >
        <p class="truncate text-sm text-stone-800">{{ toastMessage }}</p>
      </div>
    </transition>
  </teleport>
</template>