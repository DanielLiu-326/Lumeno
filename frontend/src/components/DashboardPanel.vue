<script setup>
import { computed } from "vue";
import { t } from "../i18n/index.js";
import { formatSize, progressPercent, statusLabel, statusTone } from "../utils/format";
import { IconAlbum, IconFolder, IconRefresh, IconTasks } from "../utils/icons";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  data: { type: Object, default: null },
  loading: { type: Boolean, default: false },
  error: { type: String, default: "" },
});

const emit = defineEmits(["refresh", "navigate", "select-album"]);

const summary = computed(() => props.data?.summary || {});
const statItems = computed(() => [
  { label: t("dashboard.albums"), value: summary.value.own_album_count || 0, note: t("dashboard.sharedCount", { count: summary.value.shared_album_count || 0 }) },
  { label: t("dashboard.photos"), value: summary.value.accessible_photo_count || 0, note: t("dashboard.datedCount", { count: summary.value.dated_photo_count || 0 }) },
  { label: t("dashboard.storage"), value: formatSize(summary.value.total_bytes || 0), note: t("dashboard.accessibleStorage") },
  { label: t("dashboard.activeTasks"), value: summary.value.active_task_count || 0, note: t("dashboard.taskStatus") },
  { label: t("dashboard.uptime"), value: formatUptime(summary.value.uptime_seconds || 0), note: t("dashboard.currentProcess") },
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

function thumbUrl(photo, size = 360) {
  if (!photo?.path) return "";
  const owner = photo.owner ? `&owner=${encodeURIComponent(photo.owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(photo.path)}?size=${size}${owner}`;
}

function albumCover(album) {
  if (!album?.cover) return "";
  const owner = album.owner ? `&owner=${encodeURIComponent(album.owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(album.cover)}?size=320${owner}`;
}

function openAlbum(item) {
  const album = item?.album;
  if (album) emit("select-album", album);
}
</script>

<template>
  <section class="app-scrollbar h-full overflow-y-auto px-1 pb-8">
    <div class="mx-auto w-full max-w-[1500px]">
      <header class="flex flex-wrap items-end justify-between gap-4 py-2 md:py-4">
        <div>
          <p class="text-xs font-semibold uppercase text-amber-700">Lumeno</p>
          <h1 class="mt-1 text-2xl font-bold md:text-3xl" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.title") }}</h1>
          <p class="mt-1 text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("dashboard.subtitle") }}</p>
        </div>
        <button class="inline-flex h-10 items-center gap-2 rounded-lg px-3 text-sm font-medium transition"
          :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-white text-stone-700 shadow-sm hover:bg-stone-50'"
          :disabled="loading" @click="emit('refresh')">
          <IconRefresh class="h-4 w-4" :class="loading ? 'animate-spin' : ''" />
          {{ t("dashboard.refresh") }}
        </button>
      </header>

      <div v-if="error" class="my-3 border-l-4 border-rose-500 bg-rose-50 px-4 py-3 text-sm text-rose-800">
        {{ error }}
      </div>

      <div v-if="loading && !data" class="flex min-h-80 items-center justify-center">
        <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent"></div>
      </div>

      <template v-else>
        <section class="grid grid-cols-2 gap-px overflow-hidden rounded-lg border md:grid-cols-5"
          :class="isDark ? 'border-stone-700 bg-stone-700' : 'border-stone-200 bg-stone-200'">
          <div v-for="item in statItems" :key="item.label" class="min-w-0 px-4 py-4"
            :class="isDark ? 'bg-stone-900/90' : 'bg-white/80'">
            <p class="truncate text-xs font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ item.label }}</p>
            <p class="mt-1 truncate text-xl font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'" :title="String(item.value)">{{ item.value }}</p>
            <p class="mt-1 truncate text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ item.note }}</p>
          </div>
        </section>

        <section class="mt-5 border-y py-4" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
          <div class="flex flex-wrap items-center gap-2">
            <p class="mr-2 text-sm font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-800'">{{ t("dashboard.quickActions") }}</p>
            <button class="inline-flex h-9 items-center gap-2 rounded-lg bg-amber-600 px-3 text-sm font-medium text-white hover:bg-amber-700" @click="emit('navigate', 'albums')">
              <IconAlbum class="h-4 w-4" />{{ t("dashboard.openAlbums") }}
            </button>
            <button class="inline-flex h-9 items-center gap-2 rounded-lg px-3 text-sm font-medium"
              :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'" @click="emit('navigate', 'files')">
              <IconFolder class="h-4 w-4" />{{ t("dashboard.openFiles") }}
            </button>
          </div>
        </section>

        <div class="mt-6 grid gap-7 xl:grid-cols-[minmax(0,2fr)_minmax(18rem,1fr)]">
          <div class="min-w-0 space-y-7">
            <section>
              <div class="mb-3 flex items-center justify-between">
                <h2 class="text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.recentPhotos") }}</h2>
                <button class="text-xs font-medium text-amber-700" @click="emit('navigate', 'albums')">{{ t("dashboard.viewAll") }}</button>
              </div>
              <div v-if="data?.recent_photos?.length" class="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-6">
                <button v-for="photo in data.recent_photos" :key="`${photo.owner}:${photo.path}`"
                  class="group relative aspect-square overflow-hidden rounded-md bg-stone-200" @click="openAlbum(photo)">
                  <img :src="thumbUrl(photo)" alt="" loading="lazy" class="h-full w-full object-cover transition duration-300 group-hover:scale-105" />
                  <span class="absolute inset-x-0 bottom-0 truncate bg-black/60 px-2 py-1 text-left text-[10px] text-white">{{ photo.album_name }}</span>
                </button>
              </div>
              <p v-else class="py-10 text-center text-sm" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t("dashboard.noPhotos") }}</p>
            </section>

            <section>
              <h2 class="mb-3 text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.recentAlbums") }}</h2>
              <div v-if="data?.recent_albums?.length" class="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                <button v-for="album in data.recent_albums" :key="album.album" class="flex min-w-0 items-center gap-3 rounded-md border p-2 text-left"
                  :class="isDark ? 'border-stone-700 bg-stone-900/70' : 'border-stone-200 bg-white/70'" @click="openAlbum(album)">
                  <div class="h-14 w-14 shrink-0 overflow-hidden rounded bg-stone-200">
                    <img v-if="albumCover(album)" :src="albumCover(album)" alt="" class="h-full w-full object-cover" />
                  </div>
                  <div class="min-w-0">
                    <p class="truncate text-sm font-semibold">{{ album.name }}</p>
                    <p class="mt-1 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("dashboard.photoCount", { count: album.photo_count }) }}</p>
                  </div>
                </button>
              </div>
              <p v-else class="py-8 text-center text-sm" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t("dashboard.noAlbums") }}</p>
            </section>

            <section class="grid gap-6 lg:grid-cols-2">
              <div v-for="group in [{ key: 'on', title: t('dashboard.onThisDay'), photos: data?.on_this_day?.photos }, { key: 'daily', title: t('dashboard.dailyPicks'), photos: data?.daily_picks?.photos }]" :key="group.key">
                <h2 class="mb-3 text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ group.title }}</h2>
                <div v-if="group.photos?.length" class="grid grid-cols-4 gap-1.5">
                  <button v-for="photo in group.photos.slice(0, 4)" :key="`${group.key}:${photo.owner}:${photo.path}`" class="aspect-square overflow-hidden rounded bg-stone-200" @click="openAlbum(photo)">
                    <img :src="thumbUrl(photo, 240)" alt="" loading="lazy" class="h-full w-full object-cover" />
                  </button>
                </div>
                <p v-else class="py-6 text-sm" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t("dashboard.noMemories") }}</p>
              </div>
            </section>
          </div>

          <aside class="min-w-0 space-y-7">
            <section>
              <div class="mb-3 flex items-center gap-2">
                <IconTasks class="h-4 w-4 text-amber-600" />
                <h2 class="text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.activeTasks") }}</h2>
              </div>
              <div v-if="data?.active_tasks?.length" class="divide-y" :class="isDark ? 'divide-stone-700' : 'divide-stone-200'">
                <div v-for="task in data.active_tasks" :key="task.id" class="py-3 first:pt-0">
                  <div class="flex items-center justify-between gap-2">
                    <p class="min-w-0 truncate text-sm font-medium">{{ task.name }}</p>
                    <span class="shrink-0 rounded px-2 py-0.5 text-[10px]" :class="statusTone(task.status)">{{ statusLabel(task.status) }}</span>
                  </div>
                  <div class="mt-2 h-1.5 overflow-hidden rounded-full" :class="isDark ? 'bg-stone-700' : 'bg-stone-200'">
                    <div class="h-full bg-amber-600" :style="{ width: `${progressPercent(task)}%` }"></div>
                  </div>
                </div>
              </div>
              <p v-else class="py-6 text-sm" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t("dashboard.noActiveTasks") }}</p>
            </section>

            <section v-if="data?.admin" class="border-t pt-5" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
              <h2 class="text-base font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t("dashboard.adminOverview") }}</h2>
              <dl class="mt-3 grid grid-cols-2 gap-2 text-sm">
                <div class="rounded-md p-3" :class="isDark ? 'bg-stone-900/70' : 'bg-white/70'">
                  <dt :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("dashboard.users") }}</dt>
                  <dd class="mt-1 text-xl font-bold">{{ data.admin.user_count }}</dd>
                </div>
                <div class="rounded-md p-3" :class="isDark ? 'bg-stone-900/70' : 'bg-white/70'">
                  <dt :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t("dashboard.webdavShares") }}</dt>
                  <dd class="mt-1 text-xl font-bold">{{ data.admin.webdav_share_count }}</dd>
                </div>
              </dl>
              <button class="mt-3 text-xs font-medium text-amber-700" @click="emit('navigate', 'settings')">{{ t("dashboard.openSettings") }}</button>
            </section>
          </aside>
        </div>
      </template>
    </div>
  </section>
</template>