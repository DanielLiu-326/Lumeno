<script setup>
import { computed, ref } from "vue";
import { t } from "../i18n/index.js";
import {
  WALLPAPER_PRESETS,
  normalizeWallpaperSource,
  wallpaperPreviewStyle,
} from "../utils/wallpaper.js";

const props = defineProps({
  modelValue: { type: Object, default: () => ({ type: "preset", id: "lumen" }) },
  isDark: { type: Boolean, default: false },
  uploading: { type: Boolean, default: false },
});

const emit = defineEmits(["update:modelValue", "upload"]);

const fileInput = ref(null);
const source = computed(() => normalizeWallpaperSource(props.modelValue));

const types = [
  { id: "preset", labelKey: "settings.wallpaperPreset" },
  { id: "image", labelKey: "settings.wallpaperImage" },
  { id: "color", labelKey: "settings.wallpaperColor" },
];

function patch(next) {
  emit("update:modelValue", normalizeWallpaperSource({ ...source.value, ...next }));
}

function setType(type) {
  if (type === source.value.type) return;
  if (type === "image") {
    patch({ type: "image", file: source.value.file || "" });
    return;
  }
  if (type === "color") {
    patch({ type: "color", hex: source.value.hex || "#f4efe7" });
    return;
  }
  patch({ type: "preset", id: source.value.id || "lumen" });
}

function triggerUpload() {
  fileInput.value?.click();
}

function onFileChange(event) {
  const file = event.target.files?.[0];
  event.target.value = "";
  if (!file) return;
  emit("upload", file);
}

const previewStyle = computed(() => wallpaperPreviewStyle(source.value, props.isDark));
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex rounded-lg p-0.5 text-xs font-medium" :class="isDark ? 'bg-stone-900/70' : 'bg-stone-200/80'">
      <button
        v-for="item in types"
        :key="item.id"
        type="button"
        class="flex-1 rounded-md px-2 py-1.5 transition"
        :class="source.type === item.id
          ? (isDark ? 'bg-stone-700 text-stone-50 shadow-sm' : 'bg-white text-stone-900 shadow-sm')
          : (isDark ? 'text-stone-400 hover:text-stone-200' : 'text-stone-600 hover:text-stone-800')"
        @click="setType(item.id)"
      >
        {{ t(item.labelKey) }}
      </button>
    </div>

    <div v-if="source.type === 'preset'" class="grid grid-cols-2 gap-2 sm:grid-cols-4">
      <button
        v-for="preset in WALLPAPER_PRESETS"
        :key="preset.id"
        type="button"
        class="group overflow-hidden rounded-xl border text-left transition"
        :class="source.id === preset.id
          ? (isDark ? 'border-amber-400 ring-2 ring-amber-400/40' : 'border-amber-500 ring-2 ring-amber-500/30')
          : (isDark ? 'border-stone-700 hover:border-stone-500' : 'border-stone-200 hover:border-stone-300')"
        @click="patch({ type: 'preset', id: preset.id })"
      >
        <span class="block h-14 w-full" :style="{ background: isDark ? preset.dark : preset.light }"></span>
        <span class="block px-2 py-1.5 text-[11px]" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
          {{ t(`settings.wallpaperPreset_${preset.id}`) }}
        </span>
      </button>
    </div>

    <div v-else-if="source.type === 'image'" class="flex flex-col gap-2">
      <div
        class="h-28 overflow-hidden rounded-xl border"
        :class="isDark ? 'border-stone-700 bg-stone-900/50' : 'border-stone-200 bg-stone-100'"
        :style="source.file ? previewStyle : undefined"
      >
        <div v-if="!source.file" class="flex h-full items-center justify-center text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t('settings.wallpaperImageEmpty') }}
        </div>
      </div>
      <input ref="fileInput" type="file" accept="image/*" class="hidden" @change="onFileChange" />
      <button
        type="button"
        class="self-start rounded-lg px-3 py-1.5 text-xs font-medium transition"
        :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
        :disabled="uploading"
        @click="triggerUpload"
      >
        {{ uploading ? t('settings.wallpaperUploading') : t('settings.wallpaperUpload') }}
      </button>
      <p class="text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('settings.wallpaperUploadHint') }}</p>
    </div>

    <div v-else class="flex items-center gap-3">
      <input
        type="color"
        class="h-10 w-14 cursor-pointer rounded-lg border bg-transparent"
        :class="isDark ? 'border-stone-700' : 'border-stone-300'"
        :value="source.hex || '#f4efe7'"
        @input="patch({ type: 'color', hex: $event.target.value })"
      />
      <input
        type="text"
        class="w-28 rounded-lg border px-2 py-1.5 font-mono text-xs"
        :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100' : 'border-stone-300 bg-white text-stone-900'"
        :value="source.hex || '#f4efe7'"
        maxlength="7"
        @change="patch({ type: 'color', hex: $event.target.value })"
      />
    </div>
  </div>
</template>
