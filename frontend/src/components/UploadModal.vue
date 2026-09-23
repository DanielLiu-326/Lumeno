<script setup>
import { ref } from "vue";
import { IconClose, IconCloudUpload } from "../utils/icons";
import { t } from "../i18n/index.js";
import AppDialog from "./AppDialog.vue";

defineProps({
  show: {
    type: Boolean,
    default: true,
  },
  busyUpload: {
    type: Boolean,
    required: true,
  },
  isDark: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits(["close", "upload-files", "upload-directory"]);

const fileInput = ref(null);
const directoryInput = ref(null);
const isDragOver = ref(false);

function triggerUpload(isDirectory) {
  const input = isDirectory ? directoryInput.value : fileInput.value;
  if (!input) {
    return;
  }

  input.value = "";
  input.click();
}

function handleFileSelection(event) {
  const files = Array.from(event.target.files || []);
  if (files.length > 0) {
    emit("upload-files", files);
    emit("close");
  }
  event.target.value = "";
}

function handleDirectorySelection(event) {
  const files = Array.from(event.target.files || []);
  if (files.length > 0) {
    emit("upload-directory", files);
    emit("close");
  }
  event.target.value = "";
}

function handleDrop(event) {
  event.preventDefault();
  isDragOver.value = false;
  const files = Array.from(event.dataTransfer.files || []);
  if (files.length > 0) {
    emit("upload-files", files);
    emit("close");
  }
}
</script>

<template>
  <AppDialog :show="show" :z-index="50" @close="emit('close')">
      <div
        class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-[2rem] border p-6"
        :class="isDark ? 'text-stone-100' : ''"
      >
        <div class="mb-5 flex items-center justify-between">
          <h3 class="text-lg font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-900'">{{ t('upload.title') }}</h3>
          <button
            class="flex h-8 w-8 items-center justify-center rounded-full transition"
            :class="isDark ? 'text-stone-400 hover:bg-stone-800 hover:text-stone-200' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-700'"
            @click="emit('close')"
          >
            <IconClose class="h-5 w-5" />
          </button>
        </div>

        <div
          class="mb-5 flex h-44 flex-col items-center justify-center rounded-2xl border-2 border-dashed transition"
          :class="isDragOver
            ? (isDark ? 'border-amber-500 bg-amber-900/20' : 'border-amber-500 bg-amber-50')
            : (isDark ? 'border-stone-700 bg-stone-800/80' : 'border-stone-300 bg-stone-50')"
          @dragover.prevent="isDragOver = true"
          @dragleave.self="isDragOver = false"
          @drop="handleDrop"
        >
          <IconCloudUpload class="mb-3 h-10 w-10" :class="isDark ? 'text-stone-400' : 'text-stone-400'" />
          <p class="pointer-events-none text-sm" :class="isDark ? 'text-stone-300' : 'text-stone-500'">
            {{ isDragOver ? t('upload.dropRelease') : t('upload.dropHere') }}
          </p>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <button
            class="rounded-2xl px-4 py-3 text-sm font-medium transition hover:-translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-60"
            :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
            :disabled="busyUpload"
            @click="triggerUpload(true)"
          >
            {{ t('upload.uploadFolder') }}
          </button>
          <button
            class="rounded-2xl px-4 py-3 text-sm font-medium transition hover:-translate-y-0.5 disabled:cursor-not-allowed disabled:opacity-60"
            :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
            :disabled="busyUpload"
            @click="triggerUpload(false)"
          >
            {{ t('upload.uploadFiles') }}
          </button>
        </div>
      </div>
  </AppDialog>

  <input ref="fileInput" hidden multiple type="file" @change="handleFileSelection" />
  <input
    ref="directoryInput"
    hidden
    multiple
    type="file"
    webkitdirectory
    @change="handleDirectorySelection"
  />
</template>
