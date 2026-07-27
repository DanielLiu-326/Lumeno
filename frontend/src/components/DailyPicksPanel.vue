<script setup>
import { ref, computed, onMounted } from "vue";
import { t } from "../i18n/index.js";

const props = defineProps({
  isDark: { type: Boolean, default: false },
});

const photos = ref([]);
const loading = ref(true);
const lightbox = ref(null); // { index }

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
  return `/api/thumbnail/${encodeURIComponent(photo.path)}?size=${size}${ownerParam}`;
}

function openLightbox(idx) {
  lightbox.value = { index: idx };
}
function closeLightbox() {
  lightbox.value = null;
}
function navigateLightbox(dir) {
  if (!lightbox.value) return;
  const next = lightbox.value.index + dir;
  if (next >= 0 && next < photos.value.length) {
    lightbox.value = { index: next };
  }
}
const lbPhoto = computed(() =>
  lightbox.value != null ? photos.value[lightbox.value.index] : null,
);

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
          v-for="(photo, idx) in photos"
          :key="photo.path"
          class="group relative aspect-square overflow-hidden rounded-lg focus:outline-none"
          :class="isDark ? 'bg-stone-800' : 'bg-stone-200'"
          @click="openLightbox(idx)"
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

    <!-- Lightbox -->
    <Teleport to="body">
      <div
        v-if="lightbox && lbPhoto"
        class="fixed inset-0 z-[200] flex items-center justify-center bg-black/85 backdrop-blur-sm"
        @click.self="closeLightbox"
        @keydown.esc="closeLightbox"
        @keydown.arrow-left="navigateLightbox(-1)"
        @keydown.arrow-right="navigateLightbox(1)"
        tabindex="-1"
      >
        <!-- Prev -->
        <button
          class="absolute left-3 top-1/2 -translate-y-1/2 rounded-full bg-black/50 p-2 text-white hover:bg-black/75"
          @click="navigateLightbox(-1)"
        >
          <svg xmlns="http://www.w3.org/2000/svg" class="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
          </svg>
        </button>

        <!-- Image -->
        <div class="max-h-[90vh] max-w-[90vw] flex flex-col items-center gap-3">
          <img
            :src="thumbUrl(lbPhoto, 1200)"
            class="max-h-[80vh] max-w-[90vw] rounded-xl object-contain shadow-2xl"
            alt=""
          />
          <div class="text-center text-white/80 text-sm">
            <p class="font-medium">{{ lbPhoto.album_name }} · {{ lbPhoto.date }}</p>
            <p v-if="lbPhoto.camera_model" class="text-xs text-white/50">
              {{ lbPhoto.camera_make }} {{ lbPhoto.camera_model }}
              <template v-if="lbPhoto.focal_length"> · {{ lbPhoto.focal_length }}</template>
              <template v-if="lbPhoto.aperture"> · {{ lbPhoto.aperture }}</template>
              <template v-if="lbPhoto.shutter_speed"> · {{ lbPhoto.shutter_speed }}</template>
              <template v-if="lbPhoto.iso"> · ISO {{ lbPhoto.iso }}</template>
            </p>
          </div>
        </div>

        <!-- Next -->
        <button
          class="absolute right-3 top-1/2 -translate-y-1/2 rounded-full bg-black/50 p-2 text-white hover:bg-black/75"
          @click="navigateLightbox(1)"
        >
          <svg xmlns="http://www.w3.org/2000/svg" class="h-6 w-6" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 5l7 7-7 7" />
          </svg>
        </button>

        <!-- Close -->
        <button
          class="absolute right-3 top-3 rounded-full bg-black/50 p-2 text-white hover:bg-black/75"
          @click="closeLightbox"
        >
          <svg xmlns="http://www.w3.org/2000/svg" class="h-5 w-5" fill="none" viewBox="0 0 24 24" stroke="currentColor">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
    </Teleport>
  </div>
</template>
