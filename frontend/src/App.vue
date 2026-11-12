<script setup>
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import AlbumPanel from "./components/AlbumPanel.vue";
import AppSidebar from "./components/AppSidebar.vue";
import ConflictModal from "./components/ConflictModal.vue";
import DailyPicksPanel from "./components/DailyPicksPanel.vue";
import DashboardPanel from "./components/DashboardPanel.vue";
import FileBrowserPanel from "./components/FileBrowserPanel.vue";
import LoginPanel from "./components/LoginPanel.vue";
import OnThisDayPanel from "./components/OnThisDayPanel.vue";
import SettingsPanel from "./components/SettingsPanel.vue";
import StoragePanel from "./components/StoragePanel.vue";
import { useAlbumApp } from "./composables/useAlbumApp";
import { normalizeDiscoveryPhoto } from "./utils/photoDetail.js";
import { progressPercent, statusLabel, statusTone } from "./utils/format";
import { IconAlbum, IconBell, IconCalendarDay, IconClear, IconClose, IconFolder, IconGear, IconHome, IconMenu, IconMoon, IconSparkles, IconSun, IconTasks } from "./utils/icons";
import { t, locale } from "./i18n/index.js";
import { wallpaperLayerState } from "./utils/wallpaper.js";
import { applyAccent } from "./utils/accent.js";
import { scheduleSafeAreaSync } from "./utils/safeArea.js";
import { withThemeTransition } from "./utils/themeTransition.js";
import { layoutViewportSize } from "./utils/uiScale.js";

const app = proxyRefs(useAlbumApp());
const albumPanelRef = ref(null);
const activeTab = ref("dashboard");
const settingsSection = ref("preferences");
const settingsFocus = ref(0);
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
  background: `conic-gradient(var(--color-amber-500) ${averageTaskProgress.value}%, #e7e5e4 0)`,
}));
const unreadNotificationCount = computed(
  () => notifications.value.filter((item) => !item.read).length,
);

const isDark = computed(() => theme.value === "dark");
const wallpaperLayer = computed(() => wallpaperLayerState(app.userWallpaper, isDark.value));
watch(() => app.userAccent, (next) => applyAccent(next), { immediate: true });
const activeTabTitle = computed(() => {
  if (activeTab.value === "dashboard") return t("nav.dashboard");
  if (activeTab.value === "albums") return t("nav.albums");
  if (activeTab.value === "settings") return t("nav.settings");
  if (activeTab.value === "on_this_day") return t("nav.on_this_day");
  if (activeTab.value === "daily_picks") return t("nav.daily_picks");
  return t("nav.files");
});
const activeTabIcon = computed(() => {
  if (activeTab.value === "dashboard") return IconHome;
  if (activeTab.value === "albums") return IconAlbum;
  if (activeTab.value === "settings") return IconGear;
  if (activeTab.value === "on_this_day") return IconCalendarDay;
  if (activeTab.value === "daily_picks") return IconSparkles;
  return IconFolder;
});

const THEME_COLOR = {
  light: "#f4efe7",
  dark: "#0f172a",
};

function applyThemeColor(nextTheme) {
  const color = nextTheme === "dark" ? THEME_COLOR.dark : THEME_COLOR.light;
  let meta = document.querySelector('meta[name="theme-color"]');
  if (!meta) {
    meta = document.createElement("meta");
    meta.setAttribute("name", "theme-color");
    document.head.appendChild(meta);
  }
  meta.setAttribute("content", color);
  document.documentElement.style.backgroundColor = color;
  document.body.style.backgroundColor = color;
}

function applyTheme(nextTheme, animate = false) {
  withThemeTransition(() => {
    theme.value = nextTheme;
    document.documentElement.classList.toggle("dark", nextTheme === "dark");
    localStorage.setItem("album-theme", nextTheme);
    applyThemeColor(nextTheme);
  }, { animate });
}

function syncAppViewport() {
  const { height } = layoutViewportSize();
  document.documentElement.style.setProperty("--app-height", `${height}px`);
  scheduleSafeAreaSync();
}

function hideBrowserChrome() {
  if (window.innerWidth >= 768) return;
  if (window.scrollY < 1) window.scrollTo(0, 1);
}

function onViewportChange() {
  hideBrowserChrome();
  syncAppViewport();
}

function updateViewportMode() {
  isMobile.value = window.innerWidth < 768 || window.screen.width < 768;
}

function toggleTheme() {
  applyTheme(isDark.value ? "light" : "dark", true);
}

function openTaskList() {
  taskListOpen.value = !taskListOpen.value;
  if (taskListOpen.value) {
    notificationOpen.value = false;
  }
}

watch(taskListOpen, (open) => {
  app.setTaskListExpanded(open);
});

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

function preventBrowserPageZoom(event) {
  if (event.touches && event.touches.length > 1) {
    event.preventDefault();
  }
}

function preventBrowserGesture(event) {
  event.preventDefault();
}

onMounted(() => {
  const saved = localStorage.getItem("album-theme");
  applyTheme(saved === "dark" ? "dark" : "light");
  updateViewportMode();
  syncAppViewport();
  hideBrowserChrome();
  document.addEventListener("click", onGlobalClick);
  document.addEventListener("keydown", onGlobalKeyDown);
  document.addEventListener("touchmove", preventBrowserPageZoom, { passive: false });
  document.addEventListener("touchstart", hideBrowserChrome, { passive: true });
  document.addEventListener("gesturestart", preventBrowserGesture, { passive: false });
  document.addEventListener("gesturechange", preventBrowserGesture, { passive: false });
  window.addEventListener("resize", updateViewportMode);
  window.addEventListener("resize", onViewportChange);
  window.addEventListener("orientationchange", onViewportChange);
  window.visualViewport?.addEventListener("resize", onViewportChange);
  window.visualViewport?.addEventListener("scroll", onViewportChange);
});

onBeforeUnmount(() => {
  document.removeEventListener("click", onGlobalClick);
  document.removeEventListener("keydown", onGlobalKeyDown);
  document.removeEventListener("touchmove", preventBrowserPageZoom);
  document.removeEventListener("touchstart", hideBrowserChrome);
  document.removeEventListener("gesturestart", preventBrowserGesture);
  document.removeEventListener("gesturechange", preventBrowserGesture);
  window.removeEventListener("resize", updateViewportMode);
  window.removeEventListener("resize", onViewportChange);
  window.removeEventListener("orientationchange", onViewportChange);
  window.visualViewport?.removeEventListener("resize", onViewportChange);
  window.visualViewport?.removeEventListener("scroll", onViewportChange);
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
  async (tab, prev) => {
    if (prev === "settings" && tab !== "settings") {
      await app.refreshPreferences();
    }
    if (tab === "albums") {
      await app.refreshAlbums();
    } else if (tab === "dashboard") {
      await app.refreshDashboard();
    } else if (tab === "settings") {
      await app.refreshAiStatus();
      await app.refreshStorageReport();
    }
  },
);

function switchTab(tab) {
  if (tab === "settings") {
    settingsSection.value = "preferences";
  }
  activeTab.value = tab;
}

/// Storage lives inside settings now; the dashboard tile still points at it.
function navigateTo(tab) {
  if (tab === "storage") {
    settingsSection.value = "storage";
    settingsFocus.value += 1;
    activeTab.value = "settings";
    return;
  }
  activeTab.value = tab;
}

function openAccountSettings() {
  settingsSection.value = "account";
  settingsFocus.value += 1;
  activeTab.value = "settings";
  mobileSidebarOpen.value = false;
}

async function openDashboardAlbum(album) {
  activeTab.value = "albums";
  await app.selectAlbum(album);
}

function openDiscoveryPhoto({ photo, navPhotos, fullscreen = true }) {
  const normalized = normalizeDiscoveryPhoto(photo);
  if (!normalized) return;
  albumPanelRef.value?.openPhotoDetail(normalized, {
    navPhotos: (navPhotos || []).map(normalizeDiscoveryPhoto).filter(Boolean),
    fullscreen,
  });
}
</script>

<template>
  <!-- Login: full-screen standalone page -->
  <LoginPanel
    v-if="!app.loggedIn"
    :is-dark="isDark"
    :busy-login="app.busy.login"
    :login-form="app.loginForm"
    @submit="app.login"
    @register="app.register"
  />

  <!-- Main layout: sidebar + content -->
  <div
    v-else
    class="relative flex overflow-hidden"
    :class="isDark ? 'text-stone-100' : ''"
    :style="{ height: 'var(--app-height, 100dvh)' }"
  >
    <div class="pointer-events-none absolute inset-0" :style="wallpaperLayer.style"></div>
    <div
      v-if="wallpaperLayer.showGrid"
      class="pointer-events-none absolute inset-0 opacity-25 [background-image:linear-gradient(rgba(125,98,73,0.06)_1px,transparent_1px),linear-gradient(90deg,rgba(125,98,73,0.06)_1px,transparent_1px)] [background-size:28px_28px] [mask-image:linear-gradient(to_bottom,rgba(0,0,0,0.75),transparent_88%)]"
    ></div>

    <AppSidebar
      class="relative z-10 hidden md:flex"
      :active-tab="activeTab"
      :is-dark="isDark"
      :username="app.currentUser"
      :avatar="app.userAvatar"
      @switch-tab="switchTab"
      @open-account="openAccountSettings"
      @logout="app.logout"
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
          class="app-overlay-screen flex h-full w-full"
          :active-tab="activeTab"
          :is-dark="isDark"
          :username="app.currentUser"
          :avatar="app.userAvatar"
          @switch-tab="(tab) => { switchTab(tab); mobileSidebarOpen = false; }"
          @open-account="openAccountSettings"
          @logout="app.logout"
        />
      </div>
    </Transition>

    <div class="relative z-10 flex flex-1 flex-col overflow-hidden">
      <div
        class="relative z-50 app-split-b px-4 pb-3 app-frost-toolbar"
        :style="{ paddingTop: 'calc(0.75rem + var(--safe-top, 0px))' }"
      >
        <div class="flex items-center justify-between gap-3">
          <div class="flex min-w-0 items-center gap-3">
            <button
              v-if="isMobile"
              class="app-btn app-btn-icon h-11 w-11"
              :title="t('topbar.openSidebar')"
              @click="mobileSidebarOpen = true"
            >
              <IconMenu class="h-5 w-5" />
            </button>
            <component
              v-else
              :is="activeTabIcon"
              class="h-5 w-5 shrink-0"
              :class="isDark ? 'text-stone-400' : 'text-stone-500'"
              aria-hidden="true"
            />
            <div class="min-w-0">
            <p class="text-xs font-medium uppercase tracking-[0.2em]" :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ activeTabTitle }}</p>
            </div>
          </div>

          <div class="relative flex items-center gap-2" data-topbar-menu>
            <button
              class="app-btn app-btn-icon relative h-11 w-11 md:h-10 md:w-10"
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
              class="app-btn app-btn-icon relative h-11 w-11 md:h-10 md:w-10"
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
              class="app-btn app-btn-icon h-11 w-11 md:h-10 md:w-10"
              :title="isDark ? t('topbar.switchToLight') : t('topbar.switchToDark')"
              @click="toggleTheme"
            >
              <IconSun v-if="isDark" class="h-5 w-5 text-yellow-300" />
              <IconMoon v-else class="h-5 w-5 text-yellow-700" />
            </button>

            <Teleport to="body">
            <Transition name="app-dropdown" :duration="{ enter: 300, leave: 200 }">
            <div
              v-if="notificationOpen"
              data-topbar-menu
              class="app-dropdown-surface app-frost-popover z-[90]"
              :class="isMobile
                ? 'app-overlay-screen fixed inset-0 flex flex-col border-none rounded-none'
                : 'fixed right-4 top-[calc(var(--safe-top,0px)+4.25rem)] w-[22rem] rounded-2xl'"
              @click.stop
            >
              <div class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[inherit]">
              <div
                class="app-split-b flex shrink-0 items-center justify-between px-4 py-3"
              >
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
              <div class="app-scrollbar overflow-y-auto overflow-x-hidden" :class="isMobile ? 'min-h-0 flex-1' : 'max-h-80'">
                <div
                  v-for="item in notifications"
                  :key="item.id"
                  class="app-split-b px-4 py-3 last:border-b-0"
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
            </div>
            </Transition>
            </Teleport>

            <Teleport to="body">
            <Transition name="app-dropdown" :duration="{ enter: 300, leave: 200 }">
            <div
              v-if="taskListOpen"
              data-topbar-menu
              class="app-dropdown-surface app-frost-popover z-[90]"
              :class="isMobile
                ? 'app-overlay-screen fixed inset-0 flex flex-col border-none rounded-none'
                : 'fixed right-16 top-[calc(var(--safe-top,0px)+4.25rem)] w-[26rem] rounded-2xl'"
              @click.stop
            >
              <div class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[inherit]">
              <div
                class="app-split-b flex shrink-0 items-center justify-between px-4 py-3"
              >
                <p class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t('topbar.tasks') }}</p>
                <div class="flex items-center gap-2">
                  <button
                    class="inline-flex h-9 w-9 items-center justify-center rounded-full transition"
                    :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                    :title="t('topbar.clearFinished')"
                    :aria-label="t('topbar.clearFinished')"
                    @click="clearFinishedTasksFromPanel"
                  >
                    <IconClear class="h-4 w-4" />
                  </button>
                  <button class="inline-flex h-9 w-9 items-center justify-center rounded-full transition" :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="taskListOpen = false">
                    <IconClose class="h-4 w-4" />
                  </button>
                </div>
              </div>
              <div class="app-scrollbar overflow-y-auto overflow-x-hidden" :class="isMobile ? 'min-h-0 flex-1' : 'max-h-96'">
                <div
                  v-for="task in app.tasks"
                  :key="task.id"
                  class="app-split-b px-4 py-3 last:border-b-0"
                >
                  <div class="flex min-w-0 items-start justify-between gap-3">
                    <p class="min-w-0 truncate text-sm font-medium" :class="isDark ? 'text-stone-100' : 'text-stone-900'" :title="task.name">{{ task.name }}</p>
                    <div class="flex shrink-0 items-center gap-2">
                      <button
                        v-if="task.pausable && task.status === 1"
                        class="rounded-full px-2.5 py-1 text-[11px] font-medium transition"
                        :class="isDark ? 'bg-amber-900/50 text-amber-300 hover:bg-amber-800/60' : 'bg-amber-100 text-amber-800 hover:bg-amber-200'"
                        @click="app.pauseTask(task.id)"
                      >{{ t('tasks.pause') }}</button>
                      <button
                        v-if="task.pausable && task.status === 2"
                        class="rounded-full px-2.5 py-1 text-[11px] font-medium transition"
                        :class="isDark ? 'bg-emerald-900/50 text-emerald-300 hover:bg-emerald-800/60' : 'bg-emerald-100 text-emerald-800 hover:bg-emerald-200'"
                        @click="app.resumeTask(task.id)"
                      >{{ t('tasks.resume') }}</button>
                      <button
                        v-if="task.cancellable && (task.status === 0 || task.status === 1 || task.status === 2)"
                        class="rounded-full px-2.5 py-1 text-[11px] font-medium transition"
                        :class="isDark ? 'bg-rose-900/50 text-rose-300 hover:bg-rose-800/60' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
                        @click="app.cancelTask(task.id)"
                      >{{ t('tasks.cancel') }}</button>
                      <span class="shrink-0 rounded-full px-2.5 py-1 text-[11px] font-medium" :class="statusTone(task.status)">{{ statusLabel(task.status) }}</span>
                    </div>
                  </div>
                  <div class="mt-1 flex min-w-0 items-center justify-between gap-3">
                    <p class="min-w-0 truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'" :title="task.current_task">{{ task.current_task || t('topbar.waitingForUpdate') }}</p>
                    <span class="shrink-0 text-xs font-medium tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ progressPercent(task) }}%</span>
                  </div>
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
            </Transition>
            </Teleport>
          </div>
        </div>
      </div>

      <div
        class="relative z-10 flex min-h-0 flex-1 overflow-hidden"
        :class="isMobile && activeTab === 'albums' ? 'p-0' : 'gap-5 p-3 md:p-6'"
      >
        <!-- Keep mounted so photo detail overlay works from other tabs. -->
        <div
          v-show="activeTab === 'albums'"
          class="min-h-0 min-w-0 h-full w-full"
          :class="activeTab === 'albums' ? '' : 'pointer-events-none'"
        >
          <AlbumPanel
            ref="albumPanelRef"
            :is-dark="isDark"
            :grid-paused="activeTab !== 'albums'"
            :busy-albums="app.busy.albums"
            :albums="app.albums"
            :shared-albums="app.sharedAlbums"
            :current-album="app.currentAlbum"
            :current-user="app.currentUser"
            :album-view-mode="app.albumViewMode"
            :album-photos="app.albumPhotos"
            :album-bursts="app.albumBursts"
            :collapse-bursts="app.collapseBursts"
            :burst-threshold="app.burstThreshold"
            :duplicate-groups="app.albumDuplicateGroups"
            :duplicate-total="app.albumDuplicateTotal"
            :similar-results="app.aiSimilarResults"
            :similar-query="app.aiSimilarQuery"
            :similar-loading="app.aiSimilarLoading"
            :album-people="app.albumPeople"
            :album-people-loading="app.albumPeopleLoading"
            :album-face-hints-loading="app.albumFaceHintsLoading"
            :album-face-suggestions="app.albumFaceSuggestionsDisplay"
            :album-faces-editable="app.albumFacesEditable"
            :album-faces-owner="app.albumFacesOwner"
            :album-cluster-similarity="app.albumClusterSimilarity"
            :album-cluster-reclustering="app.albumClusterReclustering"
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
            :on-search-text-in-modal="(opts) => app.searchAlbumByText({ ...opts, inModal: true })"
            :on-load-duplicates-in-modal="app.loadAlbumDuplicatesForModal"
            :fetch-photos-by-paths="app.fetchAlbumPhotosByPaths"
            @refresh-albums="app.refreshAlbums"
            @select-album="app.selectAlbum"
            @delete-album="app.deleteAlbum"
            @delete-photos="app.deleteAlbumPhotos"
            @show-duplicates="app.showAlbumDuplicates"
            @show-photos="app.showAlbumPhotos"
            @find-similar="app.findSimilarPhotos"
            @search-text="app.searchAlbumByText"
            @refresh-album-people="() => app.refreshPeopleForAlbum()"
            @patch-album-person="(id, updates) => app.patchAlbumPerson(id, updates)"
            @remove-album-center="(body) => app.removeAlbumPersonCenter(body)"
            @delete-album-person="(id) => app.deleteAlbumPerson(id)"
            @adopt-album-cluster="app.adoptAlbumCluster"
            @recluster-album-faces="app.reclusterAlbumFaces"
            @merge-album-cluster="app.mergeAlbumCluster"
            @apply-filters="app.refreshAlbumPhotos"
            @update-sort="(s) => { app.albumSort = s; app.refreshAlbumPhotos(); }"
            @import-selected="app.importSelectedToAlbum"
            @import-local="(files) => app.importLocalToAlbum(files)"
            @import-cloud="(paths) => app.importCloudToAlbum(paths)"
            @update-photo-meta="app.updatePhotoMeta"
            @copy-photo-to-album="app.copyPhotoToAlbum"
            @rebuild-index="app.rebuildAlbumIndex"
            @load-more-photos="app.loadMoreAlbumPhotos"
            @update-collapse-bursts="app.setCollapseBursts"
            @update-burst-threshold="app.setBurstThreshold"
            @select-collection="app.selectCollection"
          />
        </div>

        <template v-if="activeTab === 'dashboard'">
          <div class="min-h-0 min-w-0 w-full">
            <DashboardPanel
              :is-dark="isDark"
              :data="app.dashboard"
              :loading="app.dashboardLoading"
              :error="app.dashboardError"
              @refresh="() => app.refreshDashboard()"
              @navigate="navigateTo"
              @select-album="openDashboardAlbum"
            />
          </div>
        </template>

        <template v-else-if="activeTab === 'files'">
          <div class="min-h-0 min-w-0 w-full">
            <FileBrowserPanel
              :is-dark="isDark"
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
              @go-back="app.goBack"
              @go-forward="app.goForward"
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

        <template v-else-if="activeTab === 'settings'">
          <div class="min-h-0 min-w-0 w-full">
            <SettingsPanel
              :is-dark="isDark"
              :initial-section="settingsSection"
              :section-focus="settingsFocus"
              :members="app.members"
              :settings="app.settings"
              :ai-status="app.aiStatus"
              :is-admin="app.isAdmin"
              :current-user="app.currentUser"
              :current-locale="app.userLocale"
              :user-avatar="app.userAvatar"
              :wallpaper="app.userWallpaper"
              :accent="app.userAccent"
              :video-encode-order="app.userVideoEncodeOrder"
              :on-update-password="app.updatePassword"
              :on-upload-avatar="app.uploadAvatar"
              :on-upload-wallpaper="app.uploadWallpaper"
              :on-ai-index="app.queueAiIndex"
              :user-role="app.userRole"
              :webdav-shares="app.webdavShares"
              :on-webdav-update="app.updateWebdavShare"
              @update-settings="app.updateSettings"
              @scan-thumbnails="app.scanThumbnailsNow"
              @update-member="app.updateMember"
              @update-password="app.updatePassword"
              @update-preferences="app.updatePreferences"
              @preview-wallpaper="app.previewWallpaper"
              @preview-accent="app.previewAccent"
              @delete-member="app.deleteMember"
              @reset-member-password="({ username, new_password }) => app.resetMemberPassword(username, new_password)"
            >
              <template #storage>
                <StoragePanel
                  embedded
                  :is-dark="isDark"
                  :report="app.storageReport"
                  :loading="app.storageLoading"
                  :error="app.storageError"
                  :stale="app.storageStale"
                  :scanning="app.storageScanning"
                  :cleaning="app.storageCleaning"
                  :is-admin="app.isAdmin"
                  :members="app.storageMembers"
                  :members-loading="app.storageMembersLoading"
                  :shared-cache="app.storageSharedCache"
                  :viewing-owner="app.storageOwner"
                  :on-view-member="app.viewStorageOwner"
                  :on-scan-member="(owner) => app.startStorageScanFor(owner)"
                  :on-scan-all="app.scanAllStorageMembers"
                  :on-load-members="app.loadStorageMembers"
                  :on-refresh="app.refreshStorageReport"
                  :on-scan="app.startStorageScan"
                  :on-cleanup="app.startStorageCleanup"
                  :on-rebuild-index="app.rebuildAlbumIndex"
                />
              </template>
            </SettingsPanel>
          </div>
        </template>

        <template v-else-if="activeTab === 'on_this_day'">
          <div class="min-h-0 min-w-0 w-full h-full">
            <OnThisDayPanel :is-dark="isDark" @open-photo="openDiscoveryPhoto" />
          </div>
        </template>

        <template v-else-if="activeTab === 'daily_picks'">
          <div class="min-h-0 min-w-0 w-full h-full">
            <DailyPicksPanel :is-dark="isDark" @open-photo="openDiscoveryPhoto" />
          </div>
        </template>
      </div>

      <!-- Conflict resolution modal -->
      <ConflictModal
        :show="app.conflictModalVisible"
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
            class="fixed left-1/2 z-[140] w-[min(32rem,calc(100%-2rem))] -translate-x-1/2 rounded-xl app-frost app-card px-4 py-3 text-left shadow-2xl"
            :style="{ top: 'calc(var(--safe-top, 0px) + 1rem)' }"
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
        class="fixed left-1/2 z-[140] w-[min(32rem,calc(100%-2rem))] -translate-x-1/2 rounded-xl border border-stone-200/60 bg-white/70 px-4 py-3 text-left shadow-2xl app-frost"
        :style="{ top: 'calc(var(--safe-top, 0px) + 1rem)' }"
        :title="toastMessage"
      >
        <p class="truncate text-sm text-stone-800">{{ toastMessage }}</p>
      </div>
    </transition>
  </teleport>
</template>