<script setup>
import { IconClose } from "../utils/icons";
import { t } from "../i18n/index.js";
import AppDialog from "./AppDialog.vue";

defineProps({
  show: {
    type: Boolean,
    default: true,
  },
  isDark: {
    type: Boolean,
    default: false,
  },
});

const emit = defineEmits(["choose", "cancel"]);
</script>

<template>
  <AppDialog :show="show" :z-index="120" @close="emit('cancel')">
      <div
        class="app-dialog-panel app-dialog-panel-dense w-full max-w-sm rounded-[2rem] border p-6"
        :class="isDark ? 'text-stone-100' : 'text-stone-900'"
      >
        <!-- Header -->
        <div class="mb-4 flex items-center justify-between">
          <h3 class="text-base font-semibold">{{ t('conflict.title') }}</h3>
          <button
            class="flex h-8 w-8 items-center justify-center rounded-full transition"
            :class="isDark ? 'text-stone-400 hover:bg-stone-800 hover:text-stone-200' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-700'"
            @click="emit('cancel')"
          >
            <IconClose class="h-4 w-4" />
          </button>
        </div>

        <p class="mb-5 text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
          {{ t('conflict.description') }}
        </p>

        <div class="flex flex-col gap-3">
          <!-- Rename -->
          <button
            class="flex w-full items-start gap-3 rounded-xl border px-4 py-3 text-left transition"
            :class="isDark ? 'border-stone-700 hover:border-amber-500 hover:bg-stone-800' : 'border-stone-200 hover:border-amber-500 hover:bg-amber-50'"
            @click="emit('choose', 'rename')"
          >
            <div class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-amber-500 text-white text-xs font-bold">R</div>
            <div>
              <p class="text-sm font-medium">{{ t('conflict.rename') }}</p>
              <p class="mt-0.5 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('conflict.renameDesc') }}</p>
            </div>
          </button>

          <!-- Overwrite -->
          <button
            class="flex w-full items-start gap-3 rounded-xl border px-4 py-3 text-left transition"
            :class="isDark ? 'border-stone-700 hover:border-rose-500 hover:bg-stone-800' : 'border-stone-200 hover:border-rose-500 hover:bg-rose-50'"
            @click="emit('choose', 'overwrite')"
          >
            <div class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-rose-500 text-white text-xs font-bold">O</div>
            <div>
              <p class="text-sm font-medium">{{ t('conflict.overwrite') }}</p>
              <p class="mt-0.5 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('conflict.overwriteDesc') }}</p>
            </div>
          </button>

          <!-- Skip -->
          <button
            class="flex w-full items-start gap-3 rounded-xl border px-4 py-3 text-left transition"
            :class="isDark ? 'border-stone-700 hover:border-teal-500 hover:bg-stone-800' : 'border-stone-200 hover:border-teal-500 hover:bg-teal-50'"
            @click="emit('choose', 'skip')"
          >
            <div class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full bg-teal-500 text-white text-xs font-bold">S</div>
            <div>
              <p class="text-sm font-medium">{{ t('conflict.skip') }}</p>
              <p class="mt-0.5 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('conflict.skipDesc') }}</p>
            </div>
          </button>
        </div>
      </div>
  </AppDialog>
</template>
