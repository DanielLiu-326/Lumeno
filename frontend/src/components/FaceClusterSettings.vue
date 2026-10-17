<script setup>
import { computed, ref, watch } from "vue";
import { t } from "../i18n/index.js";
import AppDialog from "./AppDialog.vue";
import AppSlider from "./AppSlider.vue";

const MIN = 45;
const MAX = 90;
const DEFAULT = 62;

const props = defineProps({
  isDark: { type: Boolean, default: false },
  similarity: { type: Number, default: 0.62 },
  busy: { type: Boolean, default: false },
});

const emit = defineEmits(["recluster"]);

const open = ref(false);
const draft = ref(DEFAULT);

function toPercent(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return DEFAULT;
  return Math.min(MAX, Math.max(MIN, Math.round(n * 100)));
}

const saved = computed(() => toPercent(props.similarity));
const changed = computed(() => draft.value !== saved.value);

watch(
  () => props.similarity,
  (value) => {
    draft.value = toPercent(value);
  },
  { immediate: true },
);

function onRecluster() {
  if (!changed.value || props.busy) return;
  emit("recluster", draft.value / 100);
}
</script>

<template>
  <button
    type="button"
    class="rounded-lg px-2 py-1 text-[10px] font-medium"
    :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-600 hover:bg-stone-200'"
    @click="open = true"
  >
    {{ t("people.clusterSettings") }}
  </button>

  <AppDialog :show="open" :z-index="130" @close="open = false">
    <div
      class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-4"
      :class="isDark ? 'text-stone-100' : 'text-stone-900'"
    >
      <h3 class="mb-1 text-sm font-semibold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
        {{ t("people.clusterSettings") }}
      </h3>
      <p class="mb-4 text-xs leading-relaxed" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
        {{ t("people.clusterSimilarityHint") }}
      </p>
      <div class="mb-2 flex items-center justify-between gap-3 text-xs">
        <span :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t("people.clusterSimilarity") }}</span>
        <span class="font-semibold tabular-nums">{{ draft }}%</span>
      </div>
      <AppSlider
        v-model="draft"
        :min="MIN"
        :max="MAX"
        :step="1"
        :tick="saved"
        :tick-title="t('people.clusterSimilaritySaved', { value: saved })"
        :is-dark="isDark"
        :disabled="busy"
        :aria-label="t('people.clusterSimilarity')"
      />
      <div class="mt-4 flex justify-end gap-2">
        <button
          type="button"
          class="rounded-xl px-3 py-1.5 text-xs font-medium"
          :class="isDark ? 'bg-stone-800 text-stone-200' : 'bg-stone-200 text-stone-700'"
          @click="open = false"
        >
          {{ t("albums.cancel") }}
        </button>
        <button
          type="button"
          class="rounded-xl bg-amber-600 px-3 py-1.5 text-xs font-medium text-white disabled:cursor-not-allowed disabled:opacity-40"
          :disabled="!changed || busy"
          @click="onRecluster"
        >
          {{ busy ? t("people.reclustering") : t("people.recluster") }}
        </button>
      </div>
    </div>
  </AppDialog>
</template>
