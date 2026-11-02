<script setup>
import { computed } from "vue";
import { t } from "../i18n/index.js";
import { formatSize, progressPercent, statusLabel, statusTone } from "../utils/format";
import { albumCoverUrl } from "../utils/albumCover.js";
import { photoThumbnailUrl } from "../utils/thumbnailUrl.js";
import {
  IconAlbum,
  IconDatabase,
  IconFolder,
  IconImage,
  IconRefresh,
  IconSparkles,
  IconTasks,
} from "../utils/icons";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  data: { type: Object, default: null },
  loading: { type: Boolean, default: false },
  error: { type: String, default: "" },
});

const emit = defineEmits(["refresh", "navigate", "select-album"]);

const summary = computed(() => props.data?.summary || {});
const statItems = computed(() => [
  { icon: IconAlbum, label: t("dashboard.albums"), value: summary.value.own_album_count || 0, note: t("dashboard.sharedCount", { count: summary.value.shared_album_count || 0 }) },
  { icon: IconImage, label: t("dashboard.photos"), value: summary.value.accessible_photo_count || 0, note: t("dashboard.datedCount", { count: summary.value.dated_photo_count || 0 }) },
  { icon: IconDatabase, label: t("dashboard.storage"), value: formatSize(summary.value.total_bytes || 0), note: t("dashboard.accessibleStorage"), tab: "storage" },
  { icon: IconTasks, label: t("dashboard.activeTasks"), value: summary.value.active_task_count || 0, note: t("dashboard.taskStatus") },
  { icon: IconSparkles, label: t("dashboard.uptime"), value: formatUptime(summary.value.uptime_seconds || 0), note: t("dashboard.currentProcess") },
]);

function formatUptime(seconds) {
  const totalMinutes = Math.floor(Number(seconds) / 60);
  if (totalMinutes < 1) return t("dashboard.lessThanMinute");
  const days = Math.floor(totalMinutes / 1440);
  const hours = Math.floor((totalMinutes % 1440) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return t("dashboard.uptimeDaysHours", { days, hours });
  if (hours > 0) return t("dashboard.uptimeHoursMinutes", { hours, minutes });
  return t("dashboard.uptimeMinutes", { minutes });
}

function albumCover(album) {
  return albumCoverUrl(album, 320);
}

function albumTitle(item) {
  return item?.album_name || item?.name || item?.dir_name || "";
}

function openAlbum(item) {
  const album = item?.album || item?.dir_name;
  if (album) emit("select-album", album);
}

const memoryGroups = computed(() => {
  const daily = { key: "daily", title: t("dashboard.dailyPicks"), photos: props.data?.daily_picks?.photos };
  const onThisDay = { key: "on", title: t("dashboard.onThisDay"), photos: props.data?.on_this_day?.photos };
  return onThisDay.photos?.length ? [onThisDay, daily] : [daily, onThisDay];
});
</script>

<template>
  <section class="app-scrollbar h-full overflow-y-auto px-1 pb-8">
    <div class="mx-auto w-full max-w-[1500px]">
      <header class="flex flex-wrap items-end justify-between gap-3 py-2 md:py-4">
        <div class="app-dash-island max-w-xl px-4 py-3">
          <p class="app-accent-ink text-xs font-semibold uppercase tracking-[0.18em]">Lumeno</p>
          <h1 class="mt-1 font-display text-2xl font-semibold md:text-3xl" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.title") }}</h1>
          <p class="mt-1 text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("dashboard.subtitle") }}</p>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <button class="app-btn app-btn-primary h-10 px-3.5 text-sm" @click="emit('navigate', 'albums')">
            <IconAlbum class="h-4 w-4" />{{ t("dashboard.openAlbums") }}
          </button>
          <button class="app-btn app-btn-secondary h-10 px-3.5 text-sm" @click="emit('navigate', 'files')">
            <IconFolder class="h-4 w-4" />{{ t("dashboard.openFiles") }}
          </button>
          <button
            type="button"
            class="app-btn app-btn-secondary relative z-20 h-10 px-3.5 text-sm"
            @click="emit('refresh')"
          >
            <IconRefresh class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
            {{ t("dashboard.refresh") }}
          </button>
        </div>
      </header>

      <div v-if="error" class="app-dash-island my-3 border-l-4 border-rose-500 px-4 py-3 text-sm text-rose-800">
        {{ error }}
      </div>

      <div v-if="loading && !data" class="flex min-h-80 items-center justify-center">
        <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent"></div>
      </div>

      <template v-else>
        <section class="grid grid-cols-2 gap-3 md:grid-cols-5">
          <div
            v-for="item in statItems"
            :key="item.label"
            class="app-dash-island min-w-0 px-4 py-4"
            :class="item.tab ? 'cursor-pointer transition hover:ring-1 hover:ring-amber-400/60' : ''"
            @click="item.tab && emit('navigate', item.tab)"
          >
            <p class="flex items-center gap-1.5 truncate text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
              <component :is="item.icon" class="app-accent h-3.5 w-3.5 shrink-0" />
              {{ item.label }}
            </p>
            <p class="mt-1.5 truncate text-xl font-bold" :class="isDark ? 'text-stone-50' : 'text-stone-900'" :title="String(item.value)">{{ item.value }}</p>
            <p class="mt-1 truncate text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ item.note }}</p>
          </div>
        </section>

        <div class="mt-6 grid gap-6 xl:grid-cols-[minmax(0,2fr)_minmax(18rem,1fr)]">
          <div class="min-w-0 space-y-6">
            <section>
              <div class="mb-3 flex items-center justify-between gap-2">
                <h2 class="app-dash-chip text-sm font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.recentPhotos") }}</h2>
                <button class="app-dash-chip text-xs font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'" @click="emit('navigate', 'albums')">{{ t("dashboard.viewAll") }}</button>
              </div>
              <div v-if="data?.recent_photos?.length" class="grid grid-cols-3 gap-2.5 sm:grid-cols-4 lg:grid-cols-6">
                <button v-for="photo in data.recent_photos" :key="`${photo.owner}:${photo.path}`"
                  class="group relative aspect-square overflow-hidden rounded-xl bg-stone-200/70" @click="openAlbum(photo)">
                  <img :src="photoThumbnailUrl(photo)" alt="" loading="lazy" class="h-full w-full object-cover transition duration-300 group-hover:scale-105" />
                  <span class="absolute inset-x-0 bottom-0 truncate bg-black/55 px-2 py-1 text-left text-[10px] text-white">{{ albumTitle(photo) }}</span>
                </button>
              </div>
              <p v-else class="app-dash-island px-4 py-8 text-center text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("dashboard.noPhotos") }}</p>
            </section>

            <section class="app-dash-island p-4">
              <h2 class="mb-3 text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.recentAlbums") }}</h2>
              <div v-if="data?.recent_albums?.length" class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                <button v-for="album in data.recent_albums" :key="album.album"
                  class="flex min-w-0 items-center gap-3 rounded-xl p-2 text-left transition"
                  :class="isDark ? 'hover:bg-white/8' : 'hover:bg-black/4'"
                  @click="openAlbum(album)">
                  <div class="h-14 w-14 shrink-0 overflow-hidden rounded-lg bg-stone-200">
                    <img v-if="albumCover(album)" :src="albumCover(album)" alt="" class="h-full w-full object-cover" />
                  </div>
                  <div class="min-w-0">
                    <p class="truncate text-sm font-semibold">{{ albumTitle(album) }}</p>
                    <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ t("dashboard.photoCount", { count: album.photo_count }) }}</p>
                  </div>
                </button>
              </div>
              <p v-else class="py-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ t("dashboard.noAlbums") }}</p>
            </section>

            <section class="grid gap-4 lg:grid-cols-2">
              <div v-for="group in memoryGroups" :key="group.key">
                <h2 class="app-dash-chip mb-3 text-sm font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ group.title }}</h2>
                <div v-if="group.photos?.length" class="grid grid-cols-4 gap-2">
                  <button v-for="photo in group.photos.slice(0, 4)" :key="`${group.key}:${photo.owner}:${photo.path}`" class="aspect-square overflow-hidden rounded-xl bg-stone-200/70" @click="openAlbum(photo)">
                    <img :src="photoThumbnailUrl(photo, { size: 240 })" alt="" loading="lazy" class="h-full w-full object-cover" />
                  </button>
                </div>
                <p v-else class="app-dash-island px-4 py-6 text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("dashboard.noMemories") }}</p>
              </div>
            </section>
          </div>

          <aside class="min-w-0 space-y-4">
            <section class="app-dash-island p-4">
              <div class="mb-3 flex items-center gap-2">
                <IconTasks class="app-accent h-4 w-4" />
                <h2 class="text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.activeTasks") }}</h2>
              </div>
              <div v-if="data?.active_tasks?.length" class="divide-y" :class="isDark ? 'divide-white/10' : 'divide-stone-300/70'">
                <div v-for="task in data.active_tasks" :key="task.id" class="py-3 first:pt-0 last:pb-0">
                  <div class="flex items-center justify-between gap-2">
                    <p class="min-w-0 truncate text-sm font-medium">{{ task.name }}</p>
                    <span class="shrink-0 rounded px-2 py-0.5 text-[10px]" :class="statusTone(task.status)">{{ statusLabel(task.status) }}</span>
                  </div>
                  <div class="mt-1 flex min-w-0 items-center justify-between gap-2">
                    <p class="min-w-0 truncate text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-600'" :title="task.current_task">{{ task.current_task || "—" }}</p>
                    <span class="shrink-0 text-[11px] font-medium tabular-nums" :class="isDark ? 'text-stone-300' : 'text-stone-700'">{{ progressPercent(task) }}%</span>
                  </div>
                  <div class="mt-2 h-1.5 overflow-hidden rounded-full" :class="isDark ? 'bg-white/10' : 'bg-stone-200'">
                    <div class="h-full bg-amber-500" :style="{ width: `${progressPercent(task)}%` }"></div>
                  </div>
                </div>
              </div>
              <p v-else class="py-4 text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ t("dashboard.noActiveTasks") }}</p>
            </section>

            <section v-if="data?.admin" class="app-dash-island p-4">
              <h2 class="text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.adminOverview") }}</h2>
              <dl class="mt-3 grid grid-cols-2 gap-2 text-sm">
                <div class="rounded-xl px-3 py-2.5" :class="isDark ? 'bg-white/6' : 'bg-black/4'">
                  <dt :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ t("dashboard.users") }}</dt>
                  <dd class="mt-1 text-xl font-bold">{{ data.admin.user_count }}</dd>
                </div>
                <div class="rounded-xl px-3 py-2.5" :class="isDark ? 'bg-white/6' : 'bg-black/4'">
                  <dt :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ t("dashboard.webdavShares") }}</dt>
                  <dd class="mt-1 text-xl font-bold">{{ data.admin.webdav_share_count }}</dd>
                </div>
              </dl>
              <button class="app-accent mt-3 text-xs font-medium" @click="emit('navigate', 'settings')">{{ t("dashboard.openSettings") }}</button>
            </section>
          </aside>
        </div>
      </template>
    </div>
  </section>
</template>
