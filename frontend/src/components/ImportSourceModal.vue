<script setup>
import { IconClose, IconCloudUpload, IconImage } from "../utils/icons";
import { t } from "../i18n/index.js";
import AppDialog from "./AppDialog.vue";

defineProps({
  show: { type: Boolean, default: true },
  isDark: { type: Boolean, default: false },
});

const emit = defineEmits(["local", "cloud", "close"]);
</script>

<template>
  <AppDialog :show="show" :z-index="120" @close="emit('close')">
      <div
        class="app-dialog-panel app-dialog-panel-dense w-full max-w-sm rounded-[2rem] border p-6"
        :class="isDark ? 'text-stone-100' : 'text-stone-900'"
      >
        <!-- Header -->
        <div class="mb-4 flex items-center justify-between">
          <h3 class="text-base font-semibold">{{ t('import.title') }}</h3>
          <button
            class="flex h-8 w-8 items-center justify-center rounded-full transition"
            :class="isDark ? 'text-stone-400 hover:bg-stone-800 hover:text-stone-200' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-700'"
            @click="emit('close')"
          >
            <IconClose class="h-4 w-4" />
          </button>
        </div>

        <div class="flex flex-col gap-3">
          <!-- Local import -->
          <button
            class="flex w-full items-center gap-4 rounded-2xl border px-5 py-4 text-left transition"
            :class="isDark ? 'border-stone-700 hover:border-amber-500 hover:bg-stone-800' : 'border-stone-200 hover:border-amber-500 hover:bg-amber-50'"
            @click="emit('local')"
          >
            <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-amber-500/15 text-amber-500">
              <IconImage class="h-5 w-5" />
            </div>
            <div>
              <p class="text-sm font-semibold">{{ t('import.local') }}</p>
              <p class="mt-0.5 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('import.localDesc') }}</p>
            </div>
          </button>

          <!-- Cloud import -->
          <button
            class="flex w-full items-center gap-4 rounded-2xl border px-5 py-4 text-left transition"
            :class="isDark ? 'border-stone-700 hover:border-teal-500 hover:bg-stone-800' : 'border-stone-200 hover:border-teal-500 hover:bg-teal-50'"
            @click="emit('cloud')"
          >
            <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-teal-500/15 text-teal-500">
              <IconCloudUpload class="h-5 w-5" />
            </div>
            <div>
              <p class="text-sm font-semibold">{{ t('import.cloud') }}</p>
              <p class="mt-0.5 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('import.cloudDesc') }}</p>
            </div>
          </button>
        </div>
      </div>
  </AppDialog>
</template>
