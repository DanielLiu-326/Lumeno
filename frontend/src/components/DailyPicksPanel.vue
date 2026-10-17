<script setup>
import { ref, onMounted } from "vue";
import { t } from "../i18n/index.js";
import { thumbnailPathSegment } from "../utils/mediaKind.js";

const props = defineProps({
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["open-photo"]);

const photos = ref([]);
const loading = ref(true);

async function load() {
  loading.value = true;
  try {
    const res = await fetch("/api/daily-picks");
    if (res.ok) {
      const data = await res.json();
      photos.value = data.photos || [];
    }
  } finally {
    loading.value = false;
  }
}

function thumbUrl(photo, size = 360) {
  const ownerParam = photo.owner ? `&owner=${encodeURIComponent(photo.owner)}` : "";
  return `/api/thumbnail/${thumbnailPathSegment(photo.path)}?size=${size}${ownerParam}`;
}

function openPhoto(photo) {
  emit("open-photo", { photo, navPhotos: photos.value, fullscreen: true });
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
      v-else-if="photos.length === 0"
      class="flex flex-1 flex-col items-center justify-center gap-3 px-8 text-center"
    >
      <span class="text-4xl">✨</span>
      <p class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("daily_picks.empty") }}
      </p>
    </div>

    <!-- Content -->
    <div v-else class="flex-1 overflow-y-auto px-4 py-5">
      <!-- Subtitle -->
      <p
        class="mb-4 text-xs"
        :class="isDark ? 'text-stone-400' : 'text-stone-500'"
      >{{ t("daily_picks.subtitle") }}</p>

      <!-- Photo grid -->
      <div class="grid grid-cols-3 gap-1.5 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6">
        <button
          v-for="photo in photos"
          :key="photo.path"
          class="group relative aspect-square overflow-hidden rounded-lg focus:outline-none"
          :class="isDark ? 'bg-stone-800' : 'bg-stone-200'"
          @click="openPhoto(photo)"
        >
          <img
            :src="thumbUrl(photo)"
            loading="lazy"
            decoding="async"
            alt=""
            class="h-full w-full object-cover transition-transform duration-200 group-hover:scale-105"
          />
          <!-- Hover overlay -->
          <div
            class="absolute inset-x-0 bottom-0 translate-y-full bg-gradient-to-t from-black/70 to-transparent px-1.5 pb-1 pt-4 text-[10px] text-white transition-transform duration-200 group-hover:translate-y-0"
          >
            <p class="truncate">{{ photo.album_name }}</p>
            <p v-if="photo.date" class="text-white/60">{{ photo.date }}</p>
          </div>
        </button>
      </div>
    </div>
  </div>
</template>
