<script setup>
import { IconDownload, IconGrade, IconImage, IconSparkles, IconStar, IconTrash } from "../utils/icons";
import { t } from "../i18n/index.js";
import { COLOR_LABELS } from "../utils/photoCardLayout.js";
import TagChips from "./TagChips.vue";

defineProps({
  photo: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
  formatSize: { type: Function, required: true },
  dimensions: { type: String, default: "" },
  durationLabel: { type: String, default: "" },
  rating: { type: Number, default: 0 },
  colorLabelKeys: { type: Array, default: () => [] },
  tags: { type: Array, default: () => [] },
  availableTags: { type: Array, default: () => [] },
  canWrite: { type: Boolean, default: false },
  canDelete: { type: Boolean, default: false },
  isVideo: { type: Boolean, default: false },
  actions: { type: Boolean, default: true },
  showTune: { type: Boolean, default: false },
});

const emit = defineEmits([
  "download",
  "export",
  "set-rating",
  "toggle-color",
  "update:tags",
  "set-cover",
  "add-to-album",
  "find-similar",
  "delete",
  "save",
  "tune",
]);
</script>

<template>
  <div class="flex min-h-full flex-col gap-2.5 md:gap-3">
    <h3 class="text-base font-bold">{{ photo.name }}</h3>

    <div class="grid grid-cols-2 gap-x-4 gap-y-1 text-xs" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
      <div v-if="photo.datetime || photo.date">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.date') }}</span>
        {{ photo.datetime || photo.date }}
      </div>
      <div v-if="photo.camera_make || photo.camera_model">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.camera') }}</span>
        {{ [photo.camera_make, photo.camera_model].filter(Boolean).join(' ') }}
      </div>
      <div v-if="photo.lens">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.lens') }}</span>
        {{ photo.lens }}
      </div>
      <div v-if="photo.focal_length">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.focalLength') }}</span>
        {{ photo.focal_length }}
      </div>
      <div v-if="photo.aperture">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.aperture') }}</span>
        {{ photo.aperture }}
      </div>
      <div v-if="photo.shutter_speed">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.shutter') }}</span>
        {{ photo.shutter_speed }}
      </div>
      <div v-if="photo.iso">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">ISO</span>
        {{ photo.iso }}
      </div>
      <div>
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.size') }}</span>
        {{ formatSize(photo.size) }}
      </div>
      <div v-if="dimensions">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.dimensions') }}</span>
        {{ dimensions }}
      </div>
      <div v-if="durationLabel">
        <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.duration') }}</span>
        {{ durationLabel }}
      </div>
    </div>

    <hr :class="isDark ? 'border-stone-700' : 'border-stone-200'" />

    <div class="flex gap-2">
      <button class="inline-flex flex-1 items-center justify-center gap-1.5 rounded-xl px-3 py-2 text-xs font-medium transition"
        :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
        @click="emit('download')">
        <IconDownload class="h-3.5 w-3.5" />{{ t('detail.download') }}
      </button>
      <button v-if="!isVideo" class="inline-flex flex-1 items-center justify-center gap-1.5 rounded-xl px-3 py-2 text-xs font-medium transition"
        :class="isDark ? 'bg-emerald-700 text-emerald-100 hover:bg-emerald-600' : 'bg-emerald-600 text-white hover:bg-emerald-700'"
        @click="emit('export')">
        <IconDownload class="h-3.5 w-3.5" />{{ t('detail.exportBtn') }}
      </button>
    </div>

    <div>
      <p class="mb-1 text-[10px] font-semibold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.rating') }}</p>
      <div class="flex gap-1">
        <button v-for="star in 5" :key="star" class="p-0.5 transition"
          :class="star <= rating ? 'text-yellow-400' : (isDark ? 'text-stone-600 hover:text-stone-400' : 'text-stone-300 hover:text-stone-400')"
          @click="emit('set-rating', star)">
          <IconStar class="h-5 w-5" />
        </button>
      </div>
    </div>

    <div>
      <p class="mb-1 text-[10px] font-semibold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.colorLabel') }}</p>
      <div class="flex gap-2">
        <button v-for="cl in COLOR_LABELS" :key="cl.key"
          class="h-6 w-6 rounded-full transition"
          :class="[cl.color, colorLabelKeys.includes(cl.key) ? `ring-2 ${cl.ring} ring-offset-1` : 'opacity-50 hover:opacity-100']"
          :style="isDark && colorLabelKeys.includes(cl.key) ? 'ring-offset-color: #1c1917' : ''"
          @click="emit('toggle-color', cl.key)"></button>
      </div>
    </div>

    <div>
      <p class="mb-1 text-[10px] font-semibold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.tags') }}</p>
      <TagChips
        :model-value="tags"
        :available="availableTags"
        editable
        :is-dark="isDark"
        @update:model-value="emit('update:tags', $event)"
      />
    </div>

    <div v-if="actions" class="flex flex-wrap gap-2">
      <button v-if="showTune && !isVideo"
        class="inline-flex items-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
        :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
        :title="t('view.grade')"
        @click="emit('tune')">
        <IconGrade class="h-3.5 w-3.5" />{{ t('view.grade') }}
      </button>
      <button v-if="canWrite && !isVideo" class="inline-flex flex-1 items-center justify-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
        :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
        @click="emit('set-cover')">
        <IconImage class="h-3.5 w-3.5" />{{ t('detail.setAsCover') }}
      </button>
      <button
        class="inline-flex items-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
        :class="isDark ? 'bg-sky-900/60 text-sky-200 hover:bg-sky-800' : 'bg-sky-100 text-sky-700 hover:bg-sky-200'"
        @click="emit('add-to-album')">
        <IconImage class="h-3.5 w-3.5" />{{ t('detail.addToAlbum') }}
      </button>
      <button
        v-if="!isVideo"
        class="inline-flex items-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
        :class="isDark ? 'bg-violet-900/60 text-violet-200 hover:bg-violet-800' : 'bg-violet-100 text-violet-700 hover:bg-violet-200'"
        :title="t('detail.findSimilar')"
        @click="emit('find-similar')">
        <IconSparkles class="h-3.5 w-3.5" />{{ t('detail.findSimilar') }}
      </button>
      <button v-if="canDelete"
        class="inline-flex items-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
        :class="isDark ? 'bg-rose-900/60 text-rose-200 hover:bg-rose-800' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
        @click="emit('delete')">
        <IconTrash class="h-3.5 w-3.5" />{{ t('detail.delete') }}
      </button>
    </div>

    <button v-if="actions" class="mt-auto rounded-xl px-4 py-2.5 text-sm font-medium transition md:py-2"
      :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
      @click="emit('save')">
      {{ t('detail.save') }}
    </button>
    <div class="h-[var(--safe-bottom,0px)] md:hidden"></div>
  </div>
</template>
