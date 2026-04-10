<script setup>
import { ref } from "vue";
import { IconClose, IconCloudUpload } from "../utils/icons";

defineProps({
  busyUpload: {
    type: Boolean,
    required: true,
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
  <Teleport to="body">
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm"
      @click.self="emit('close')"
    >
      <div class="w-full max-w-md rounded-[2rem] border border-white/70 bg-white p-6 shadow-2xl">
        <div class="mb-5 flex items-center justify-between">
          <h3 class="text-lg font-semibold text-stone-900">上传文件</h3>
          <button
            class="flex h-8 w-8 items-center justify-center rounded-full text-stone-400 transition hover:bg-stone-200 hover:text-stone-700"
            @click="emit('close')"
          >
            <IconClose class="h-5 w-5" />
          </button>
        </div>

        <div
          class="mb-5 flex h-44 flex-col items-center justify-center rounded-2xl border-2 border-dashed transition"
          :class="isDragOver ? 'border-amber-500 bg-amber-50' : 'border-stone-300 bg-stone-50'"
          @dragover.prevent="isDragOver = true"
          @dragleave.self="isDragOver = false"
          @drop="handleDrop"
        >
          <IconCloudUpload class="mb-3 h-10 w-10 text-stone-400" />
          <p class="pointer-events-none text-sm text-stone-500">
            {{ isDragOver ? "松开以上传文件" : "拖拽文件到此处" }}
          </p>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <button
            class="rounded-2xl bg-stone-900 px-4 py-3 text-sm font-medium text-white transition hover:-translate-y-0.5 hover:bg-stone-800"
            :disabled="busyUpload"
            @click="triggerUpload(true)"
          >
            上传文件夹
          </button>
          <button
            class="rounded-2xl bg-stone-900 px-4 py-3 text-sm font-medium text-white transition hover:-translate-y-0.5 hover:bg-stone-800"
            :disabled="busyUpload"
            @click="triggerUpload(false)"
          >
            上传文件
          </button>
        </div>
      </div>
    </div>
  </Teleport>

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
