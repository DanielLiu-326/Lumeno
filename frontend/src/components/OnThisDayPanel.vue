<script setup>
import { ref, onMounted } from "vue";
import { t } from "../i18n/index.js";

const props = defineProps({
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["open-photo"]);

const groups = ref([]);   // [{ year, photos: [...] }]
const loading = ref(true);
const currentYear = new Date().getFullYear();

async function load() {
  loading.value = true;
  try {
    const res = await fetch("/api/on-this-day");
    if (res.ok) {
      const data = await res.json();
      groups.value = data.groups || [];
    }
  } finally {
    loading.value = false;
  }
}

function thumbUrl(photo, size = 360) {
  const ownerParam = photo.owner ? `&owner=${encodeURIComponent(photo.owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(photo.path)}?size=${size}${ownerParam}`;
}

function yearsAgo(year) {
  const n = currentYear - parseInt(year, 10);
  if (n === 1) return t("on_this_day.years_ago", { n });
  return t("on_this_day.years_ago_plural", { n });
}

function openPhoto(photo, group) {
  emit("open-photo", { photo, navPhotos: group.photos, fullscreen: true });
}

onMounted(load);
</script>

<template>
  <div class="relative flex h-full flex-col overflow-hidden">
    <!-- Loading -->
    <div v-if="loading" class="flex flex-1 items-center justify-center">
      <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent"></div>
    </div>

    <!-- Empty -->
    <div
      v-else-if="groups.length === 0"
      class="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center"
    >
      <span class="text-4xl">📅</span>
      <p class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("on_this_day.empty") }}
      </p>
    </div>

    <!-- Content: grouped by year, newest first -->
    <div v-else class="flex-1 overflow-y-auto px-4 py-5 space-y-8">
      <div v-for="group in groups" :key="group.year">
        <!-- Year header -->
        <div class="flex items-center gap-3 mb-3">
          <span
            class="text-xl font-bold tabular-nums"
            :class="isDark ? 'text-stone-100' : 'text-stone-800'"
          >{{ group.year }}</span>
          <span
            class="rounded-full px-2.5 py-0.5 text-xs font-medium"
            :class="isDark ? 'bg-amber-500/15 text-amber-300' : 'bg-amber-100 text-amber-700'"
          >{{ yearsAgo(group.year) }}</span>
          <span class="text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ group.photos.length }} {{ group.photos.length === 1 ? "photo" : "photos" }}
          </span>
        </div>

        <!-- Photo grid -->
        <div class="grid grid-cols-3 gap-1.5 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6">
          <button
            v-for="photo in group.photos"
            :key="photo.path"
            class="group relative aspect-square overflow-hidden rounded-lg focus:outline-none"
            :class="isDark ? 'bg-stone-800' : 'bg-stone-200'"
            @click="openPhoto(photo, group)"
          >
            <img
              :src="thumbUrl(photo)"
              loading="lazy"
              decoding="async"
              alt=""
              class="h-full w-full object-cover transition-transform duration-200 group-hover:scale-105"
            />
            <!-- Album badge on hover -->
            <div
              class="absolute inset-x-0 bottom-0 translate-y-full bg-gradient-to-t from-black/70 to-transparent px-1.5 pb-1 pt-4 text-[10px] text-white transition-transform duration-200 group-hover:translate-y-0"
            >
              <p class="truncate">{{ photo.album_name }}</p>
            </div>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
