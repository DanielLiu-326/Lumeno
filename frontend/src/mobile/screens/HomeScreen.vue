<script setup>
import { computed, inject, onMounted } from "vue";
import { useRouter } from "vue-router";
import { t } from "../../i18n/index.js";
import { formatSize } from "../../utils/format";
import { albumCoverUrl } from "../../utils/albumCover.js";
import { thumbnailPathSegment } from "../../utils/mediaKind.js";
import { albumLocation } from "../router.js";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import Screen from "../layouts/Screen.vue";
import { IconAlbum, IconFolder, IconImage, IconRefresh } from "../../utils/icons";

defineOptions({ name: "HomeScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const router = useRouter();

const summary = computed(() => app.dashboard?.summary || {});
const greeting = computed(() => {
  const hour = new Date().getHours();
  if (hour < 12) return t("mobile.greetingMorning");
  if (hour < 18) return t("mobile.greetingAfternoon");
  return t("mobile.greetingEvening");
});

function thumbUrl(photo, size = 360) {
  if (!photo?.path) return "";
  const owner = photo.owner ? `&owner=${encodeURIComponent(photo.owner)}` : "";
  return `/api/thumbnail/${thumbnailPathSegment(photo.path)}?size=${size}${owner}`;
}

function albumCover(album) {
  return albumCoverUrl(album, 480);
}

function albumTitle(item) {
  return item?.album_name || item?.name || item?.dir_name || "";
}

function openAlbum(item) {
  const dir = item?.album || item?.dir_name;
  if (!dir) return;
  app.selectAlbum(dir);
  router.push(albumLocation(dir));
}

function openDiscoveryPhoto(photo, navPhotos) {
  ui.openDiscoveryPhoto({ photo, navPhotos, fullscreen: true });
}

const discoverySections = computed(() => {
  const daily = {
    key: "daily",
    title: t("dashboard.dailyPicks"),
    photos: app.dashboard?.daily_picks?.photos || [],
    empty: t("daily_picks.empty"),
  };
  const onThisDay = {
    key: "on",
    title: t("dashboard.onThisDay"),
    photos: app.dashboard?.on_this_day?.photos || [],
    empty: t("dashboard.noMemories"),
  };
  return onThisDay.photos.length ? [onThisDay, daily] : [daily, onThisDay];
});

onMounted(() => {
  app.refreshDashboard();
});
</script>

<template>
  <Screen>
    <template #bar>
      <header class="mobile-topbar">
        <div class="min-w-0 flex-1">
          <p class="app-accent-ink text-xs font-medium uppercase tracking-[0.16em]">Lumeno</p>
          <h1 class="mt-0.5 truncate font-display text-xl font-semibold">{{ greeting }}<template v-if="app.currentUser">, {{ app.currentUser }}</template></h1>
        </div>
        <button type="button" class="mobile-icon-btn is-accent" :title="t('dashboard.refresh')" @click="app.refreshDashboard()">
          <IconRefresh class="h-5 w-5" :class="app.dashboardLoading ? 'animate-spin' : ''" />
        </button>
      </header>
    </template>

    <div class="mobile-scroll px-4 pb-6">
      <div v-if="app.dashboardError" class="mt-3 rounded-2xl border-l-4 border-rose-500 px-4 py-3 text-sm text-rose-700">
        {{ app.dashboardError }}
      </div>

      <div v-if="app.dashboardLoading && !app.dashboard" class="flex min-h-48 items-center justify-center">
        <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent"></div>
      </div>

      <template v-else>
        <section class="mt-3 grid grid-cols-2 gap-2.5">
          <button type="button" class="mobile-stat" @click="router.push('/albums')">
            <p class="mobile-stat-label"><IconAlbum class="app-accent h-3.5 w-3.5" />{{ t("dashboard.albums") }}</p>
            <p class="mobile-stat-value">{{ summary.own_album_count || 0 }}</p>
            <p class="mobile-stat-note">{{ t("dashboard.sharedCount", { count: summary.shared_album_count || 0 }) }}</p>
          </button>
          <button type="button" class="mobile-stat" @click="router.push('/albums')">
            <p class="mobile-stat-label"><IconImage class="app-accent h-3.5 w-3.5" />{{ t("dashboard.photos") }}</p>
            <p class="mobile-stat-value">{{ summary.accessible_photo_count || 0 }}</p>
            <p class="mobile-stat-note">{{ t("dashboard.datedCount", { count: summary.dated_photo_count || 0 }) }}</p>
          </button>
          <div class="mobile-stat">
            <p class="mobile-stat-label">{{ t("dashboard.storage") }}</p>
            <p class="mobile-stat-value">{{ formatSize(summary.total_bytes || 0) }}</p>
            <p class="mobile-stat-note">{{ t("dashboard.accessibleStorage") }}</p>
          </div>
          <button type="button" class="mobile-stat" @click="ui.openTasks()">
            <p class="mobile-stat-label">{{ t("dashboard.activeTasks") }}</p>
            <p class="mobile-stat-value">{{ summary.active_task_count || 0 }}</p>
            <p class="mobile-stat-note">{{ t("dashboard.taskStatus") }}</p>
          </button>
        </section>

        <div class="mt-4 grid grid-cols-2 gap-2.5">
          <button type="button" class="mobile-quick" @click="router.push('/albums')">
            <IconAlbum class="app-accent h-5 w-5" />{{ t("dashboard.openAlbums") }}
          </button>
          <button type="button" class="mobile-quick is-secondary" @click="router.push('/files')">
            <IconFolder class="app-accent h-5 w-5" />{{ t("dashboard.openFiles") }}
          </button>
        </div>

        <section class="mt-7">
          <div class="mb-3 flex items-center justify-between">
            <h2 class="text-base font-semibold">{{ t("dashboard.recentPhotos") }}</h2>
            <button type="button" class="app-accent rounded-full px-2 py-1 text-sm font-medium" @click="router.push('/albums')">{{ t("dashboard.viewAll") }}</button>
          </div>
          <div v-if="app.dashboard?.recent_photos?.length" class="mobile-h-scroll">
            <button
              v-for="photo in app.dashboard.recent_photos"
              :key="`${photo.owner}:${photo.path}`"
              type="button"
              class="mobile-cover-tile"
              @click="openAlbum(photo)"
            >
              <img :src="thumbUrl(photo)" alt="" loading="lazy" class="h-full w-full object-cover" />
              <span class="mobile-cover-caption">{{ albumTitle(photo) }}</span>
            </button>
          </div>
          <p v-else class="py-8 text-center text-sm opacity-60">{{ t("dashboard.noPhotos") }}</p>
        </section>

        <section class="mt-7">
          <h2 class="mb-3 text-base font-semibold">{{ t("dashboard.recentAlbums") }}</h2>
          <div v-if="app.dashboard?.recent_albums?.length" class="mobile-h-scroll">
            <button
              v-for="album in app.dashboard.recent_albums"
              :key="album.album"
              type="button"
              class="mobile-album-chip"
              @click="openAlbum(album)"
            >
              <div class="h-16 w-16 shrink-0 overflow-hidden rounded-xl bg-stone-200">
                <img v-if="albumCover(album)" :src="albumCover(album)" alt="" class="h-full w-full object-cover" />
              </div>
              <div class="min-w-0 text-left">
                <p class="truncate text-sm font-semibold">{{ albumTitle(album) }}</p>
                <p class="mt-1 text-xs opacity-60">{{ t("dashboard.photoCount", { count: album.photo_count }) }}</p>
              </div>
            </button>
          </div>
          <p v-else class="py-6 text-center text-sm opacity-60">{{ t("dashboard.noAlbums") }}</p>
        </section>

        <section v-for="section in discoverySections" :key="section.key" class="mt-7">
          <h2 class="mb-3 text-base font-semibold">{{ section.title }}</h2>
          <div v-if="section.photos.length" class="mobile-h-scroll">
            <button
              v-for="photo in section.photos"
              :key="`${section.key}:${photo.owner}:${photo.path}`"
              type="button"
              class="mobile-cover-tile"
              @click="openDiscoveryPhoto(photo, section.photos)"
            >
              <img :src="thumbUrl(photo)" alt="" loading="lazy" class="h-full w-full object-cover" />
            </button>
          </div>
          <p v-else class="py-6 text-center text-sm opacity-60">{{ section.empty }}</p>
        </section>
      </template>
    </div>
  </Screen>
</template>
