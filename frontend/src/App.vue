<script setup>
import { computed, onBeforeUnmount, onMounted, proxyRefs, ref, watch } from "vue";
import AppSidebar from "./components/AppSidebar.vue";
import FileBrowserPanel from "./components/FileBrowserPanel.vue";
import LoginPanel from "./components/LoginPanel.vue";
import { useAlbumApp } from "./composables/useAlbumApp";
import { progressPercent, statusLabel, statusTone } from "./utils/format";
import { IconBell, IconClose, IconMenu, IconMoon, IconPower, IconSun, IconTasks } from "./utils/icons";

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
  }
}

watch(
  () => `${app.message.kind}|${app.message.text}`,
  (next, prev) => {
    const text = app.message.text;
    if (!text || next === prev) {
      return;
    }
    const item = {
      id: Date.now() + Math.random(),
      text,
      kind: app.message.kind,
      createdAt: new Date().toLocaleTimeString("zh-CN", { hour12: false }),
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
</script>

<template>
  <!-- Login: full-screen standalone page -->
  <LoginPanel
    v-if="!app.loggedIn"
    :busy-login="app.busy.login"
    :login-form="app.loginForm"
    @submit="app.login"
  />

  <!-- Main layout: sidebar + content -->
  <div v-else class="flex h-screen overflow-hidden" :class="isDark ? 'bg-stone-950 text-stone-100' : ''">
    <AppSidebar
      class="hidden md:flex"
      :active-tab="activeTab"
      :is-dark="isDark"
      :username="app.loginForm.username"
      @switch-tab="(tab) => (activeTab = tab)"
    />

    <div
      v-if="mobileSidebarOpen"
      class="fixed inset-0 z-[70] bg-black/35 md:hidden"
      @click="mobileSidebarOpen = false"
    >
      <div class="h-full w-72" @click.stop>
        <AppSidebar
          class="flex h-full w-full"
          :active-tab="activeTab"
          :is-dark="isDark"
          :username="app.loginForm.username"
          @switch-tab="(tab) => { activeTab = tab; mobileSidebarOpen = false; }"
        />
      </div>
    </div>

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
              title="打开侧栏"
              @click="mobileSidebarOpen = true"
            >
              <IconMenu class="h-5 w-5" />
            </button>
            <div class="min-w-0">
              <p class="text-xs uppercase tracking-[0.2em] text-stone-500">文件管理</p>
            </div>
          </div>

          <div class="relative flex items-center gap-2" data-topbar-menu>
            <button
              class="relative inline-flex h-10 w-10 items-center justify-center rounded-xl transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              title="后台任务"
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
              title="通知"
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
              :title="isDark ? '切换到明亮模式' : '切换到暗黑模式'"
              @click="toggleTheme"
            >
              <IconSun v-if="isDark" class="h-5 w-5" />
              <IconMoon v-else class="h-5 w-5" />
            </button>

            <button
              class="inline-flex h-10 w-10 items-center justify-center rounded-xl bg-rose-600 text-white transition hover:bg-rose-700"
              @click="app.logout"
              title="退出登录"
            >
              <IconPower class="h-4 w-4" />
            </button>

            <div
              v-if="notificationOpen"
              class="z-40 overflow-hidden border shadow-xl md:absolute md:right-0 md:top-12 md:w-[22rem] md:rounded-2xl"
              :class="isMobile ? (isDark ? 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-stone-950' : 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-white') : (isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white')"
            >
              <div class="flex items-center justify-between border-b px-4 py-3" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
                <p class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">通知中心</p>
                <div class="flex items-center gap-2">
                  <button class="inline-flex items-center rounded-full px-3 py-1.5 text-xs font-medium transition" :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="clearNotifications">
                    清空
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
                  暂无通知
                </div>
              </div>
            </div>

            <div
              v-if="taskListOpen"
              class="z-40 overflow-hidden border shadow-xl md:absolute md:right-12 md:top-12 md:w-[26rem] md:rounded-2xl"
              :class="isMobile ? (isDark ? 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-stone-950' : 'fixed inset-0 z-[80] h-screen w-screen border-none rounded-none bg-white') : (isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white')"
            >
              <div class="flex items-center justify-between border-b px-4 py-3" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
                <p class="text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">后台任务</p>
                <div class="flex items-center gap-2">
                  <button class="inline-flex items-center rounded-full px-3 py-1.5 text-xs font-medium transition" :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'" @click="clearFinishedTasksFromPanel">
                    清理已完成
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
                    <span class="shrink-0 rounded-full px-2.5 py-1 text-[11px] font-medium" :class="statusTone(task.status)">{{ statusLabel(task.status) }}</span>
                  </div>
                  <p class="mt-1 truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'" :title="task.current_task">{{ task.current_task || '等待任务更新' }}</p>
                  <div class="mt-2 h-1.5 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-700' : 'bg-stone-200'">
                    <div
                      class="h-full rounded-full bg-linear-to-r from-amber-600 via-orange-400 to-emerald-600"
                      :style="{ width: `${progressPercent(task)}%` }"
                    ></div>
                  </div>
                </div>
                <div v-if="app.tasks.length === 0" class="px-4 py-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  暂无后台任务
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="relative z-10 flex flex-1 gap-5 overflow-hidden p-3 md:p-6">
        <!-- File management: main -->
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
              :current-path="app.currentPath"
              :entries="app.entries"
              :format-size="app.formatSize"
              :format-time="app.formatTime"
              :is-selected="app.isSelected"
              :select-all-indeterminate="app.selectAllIndeterminate"
              :selected-entries-count="app.selectedEntries.length"
              :view-mode="app.viewMode"
              @cancel-clipboard="app.cancelClipboard"
              @clear-selection="app.clearSelection"
              @copy-selected="app.copySelected"
              @delete-entry="app.deleteEntry"
              @delete-selected="app.deleteSelected"
              @download-entry="app.downloadEntry"
              @go-up="app.goUp"
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
      </div>

      <transition
        enter-active-class="transition duration-200"
        leave-active-class="transition duration-200"
        enter-from-class="translate-y-2 opacity-0"
        leave-to-class="translate-y-2 opacity-0"
      >
        <button
          v-if="toastVisible"
          class="absolute right-4 top-16 z-50 max-w-[22rem] rounded-xl border px-4 py-3 text-left shadow-lg"
          :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'"
          @click="notificationOpen = true; toastVisible = false"
          :title="toastMessage"
        >
          <p class="truncate text-sm" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ toastMessage }}</p>
          <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">点击进入通知中心</p>
        </button>
      </transition>
    </div>
  </div>
</template>