<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { IconClose, IconPlus, IconRefresh } from "../utils/icons";
import { t } from "../i18n/index.js";
import PhotoCard from "./PhotoCard.vue";
import AppDialog from "./AppDialog.vue";
import {
  MAX_BELOW_ROWS,
  OVERLAY_SLOTS,
  PHOTO_FIELDS,
  PREVIEW_PHOTO,
  addFieldToRow,
  addFieldToSlot,
  cloneLayout,
  defaultViewLayout,
  removeField,
  unusedFields,
} from "../utils/photoCardLayout.js";

const props = defineProps({
  show: { type: Boolean, default: true },
  page: { type: Boolean, default: false },
  modelValue: { type: Object, required: true },
  isDark: { type: Boolean, default: false },
  formatSize: { type: Function, required: true },
  burstThreshold: { type: Number, default: 10 },
  burstAlgorithm: { type: String, default: "dct" },
  burstEmbeddingThreshold: { type: Number, default: 80 },
  burstTimeWindow: { type: Number, default: 2 },
});

const emit = defineEmits(["update:modelValue", "update-burst-settings", "close"]);

const wrapperProps = computed(() => {
  if (props.page) return { class: "flex h-full min-h-0 flex-1 flex-col overflow-hidden" };
  return { show: props.show, zIndex: 120, centerClass: "p-3 md:p-6" };
});

const canvasRef = ref(null);
const stageRef = ref(null);
const connectors = ref([]);
const selectedSlot = ref("overlay-tl");
const addMenuFor = ref("");
const pickerTop = ref({
  "overlay-tl": 0,
  "overlay-tr": 0,
  "overlay-bl": 0,
  "overlay-br": 0,
});
const sideHeight = ref(220);
let lineRaf = 0;
let lineTimer = 0;
let ro = null;

const draft = ref(cloneLayout(props.modelValue));
const burstThresholdDraft = ref(props.burstThreshold);
const burstAlgorithmDraft = ref(props.burstAlgorithm === "embedding" ? "embedding" : "dct");
const burstEmbeddingThresholdDraft = ref(props.burstEmbeddingThreshold);
const burstTimeWindowDraft = ref(props.burstTimeWindow);
const burstTimeWindowOptions = [
  { value: 1, label: "1s" },
  { value: 2, label: "2s" },
  { value: 5, label: "5s" },
  { value: 10, label: "10s" },
  { value: 30, label: "30s" },
  { value: 60, label: "1m" },
  { value: 300, label: "5m" },
  { value: 1800, label: "30m" },
  { value: 3600, label: "1h" },
  { value: 21600, label: "6h" },
  { value: 86400, label: "24h" },
];
const layout = computed(() => draft.value);
const unused = computed(() => unusedFields(layout.value));

const aroundSlots = computed(() =>
  OVERLAY_SLOTS.map((slot) => ({
    id: slot.id,
    kind: "overlay",
    corner: slot.corner,
    label: t(slot.labelKey),
    fields: layout.value.slots?.[slot.id] || [],
  })),
);

const leftSlots = computed(() => aroundSlots.value.filter((item) => item.corner === "tl" || item.corner === "bl"));
const rightSlots = computed(() => aroundSlots.value.filter((item) => item.corner === "tr" || item.corner === "br"));

const rowItems = computed(() =>
  (layout.value.rows || []).map((fields, index) => ({
    id: `row-${index}`,
    kind: "row",
    index,
    label: t("view.belowRow", { n: index + 1 }),
    fields,
  })),
);

function fieldLabel(key) {
  return t(PHOTO_FIELDS.find((field) => field.key === key)?.labelKey || key);
}

function patch(mutator) {
  const next = cloneLayout(draft.value);
  mutator(next);
  draft.value = next;
}

function resetLayout() {
  draft.value = defaultViewLayout();
  addMenuFor.value = "";
}

function confirm() {
  emit("update:modelValue", cloneLayout(draft.value));
  if (
    burstAlgorithmDraft.value !== (props.burstAlgorithm === "embedding" ? "embedding" : "dct")
    || burstThresholdDraft.value !== props.burstThreshold
    || burstEmbeddingThresholdDraft.value !== props.burstEmbeddingThreshold
    || burstTimeWindowDraft.value !== props.burstTimeWindow
  ) {
    emit("update-burst-settings", {
      algorithm: burstAlgorithmDraft.value,
      threshold: burstThresholdDraft.value,
      embeddingThreshold: burstEmbeddingThresholdDraft.value,
      timeWindow: burstTimeWindowDraft.value,
    });
  }
  emit("close");
}

function cancel() {
  emit("close");
}

function selectPosition(id) {
  selectedSlot.value = id;
  addMenuFor.value = "";
}

function placeField(targetId, key) {
  patch((next) => {
    if (targetId.startsWith("row-")) {
      addFieldToRow(next, Number(targetId.slice(4)), key);
    } else {
      addFieldToSlot(next, targetId, key);
    }
  });
  selectedSlot.value = targetId;
  addMenuFor.value = "";
}

function dropField(key) {
  patch((next) => removeField(next, key));
}

function pickUnused(key) {
  placeField(selectedSlot.value || "overlay-tl", key);
}

function toggleAddMenu(id) {
  if (unused.value.length === 0) return;
  addMenuFor.value = addMenuFor.value === id ? "" : id;
  selectedSlot.value = id;
}

function addRow() {
  if ((layout.value.rows || []).length >= MAX_BELOW_ROWS) return;
  patch((next) => {
    next.rows.push([]);
  });
  nextTick(() => {
    selectedSlot.value = `row-${(draft.value.rows || []).length - 1}`;
  });
}

function removeRow(index) {
  patch((next) => {
    next.rows.splice(index, 1);
  });
  if (selectedSlot.value === `row-${index}`) {
    selectedSlot.value = "overlay-tl";
  }
}

function pickerClass(id) {
  return selectedSlot.value === id
    ? (props.isDark ? "border-stone-400 bg-white/10" : "border-stone-400 bg-white/70")
    : (props.isDark ? "border-white/12 bg-white/6 hover:border-white/25" : "border-stone-300/80 bg-white/45 hover:border-stone-400");
}

function layoutStage() {
  if (props.page) return;
  const canvas = canvasRef.value;
  const stage = stageRef.value;
  if (!canvas || !stage || window.innerWidth < 768) return;
  const image = canvas.querySelector("[data-preview-image]");
  const stageBox = stage.getBoundingClientRect();
  if (image) {
    sideHeight.value = Math.max(image.getBoundingClientRect().height, 160);
  }
  const nextTops = { ...pickerTop.value };
  for (const item of aroundSlots.value) {
    const slot = canvas.querySelector(`[data-preview-slot="${item.id}"]`);
    const picker = canvas.querySelector(`[data-slot-item="${item.id}"]`);
    if (!slot || !picker) continue;
    const slotBox = slot.getBoundingClientRect();
    const mid = slotBox.top + slotBox.height / 2 - stageBox.top;
    nextTops[item.id] = Math.max(0, mid - picker.offsetHeight / 2);
  }
  pickerTop.value = nextTops;
  updateConnectors();
}

function updateConnectors() {
  const canvas = canvasRef.value;
  if (props.page || !canvas || window.innerWidth < 768) {
    connectors.value = [];
    return;
  }
  const box = canvas.getBoundingClientRect();
  const next = [];
  for (const item of aroundSlots.value) {
    const picker = canvas.querySelector(`[data-slot-item="${item.id}"]`);
    const preview = canvas.querySelector(`[data-preview-slot="${item.id}"]`);
    if (!picker || !preview) continue;
    const aBox = picker.getBoundingClientRect();
    const pBox = preview.getBoundingClientRect();
    const left = item.corner === "tl" || item.corner === "bl";
    const y = pBox.top + pBox.height / 2 - box.top;
    const from = {
      x: (left ? pBox.left : pBox.right) - box.left,
      y,
    };
    const to = {
      x: (left ? aBox.right : aBox.left) - box.left,
      y,
    };
    next.push({
      id: item.id,
      from,
      to,
      d: `M ${from.x.toFixed(1)} ${from.y.toFixed(1)} L ${to.x.toFixed(1)} ${from.y.toFixed(1)}`,
      active: selectedSlot.value === item.id,
    });
  }
  connectors.value = next;
}

function scheduleLayout() {
  if (lineRaf) return;
  lineRaf = requestAnimationFrame(() => {
    lineRaf = 0;
    layoutStage();
  });
}

function onKeydown(event) {
  if (!props.show) return;
  if (event.key === "Escape") cancel();
}

watch(
  () => props.show,
  (open) => {
    if (!open) return;
    draft.value = cloneLayout(props.modelValue);
    burstThresholdDraft.value = props.burstThreshold;
    burstAlgorithmDraft.value = props.burstAlgorithm === "embedding" ? "embedding" : "dct";
    burstEmbeddingThresholdDraft.value = props.burstEmbeddingThreshold;
    burstTimeWindowDraft.value = props.burstTimeWindow;
    selectedSlot.value = "overlay-tl";
    addMenuFor.value = "";
    nextTick(layoutStage);
  },
);

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  window.addEventListener("resize", scheduleLayout);
  nextTick(() => {
    layoutStage();
    lineTimer = window.setTimeout(layoutStage, 80);
    ro = new ResizeObserver(scheduleLayout);
    if (canvasRef.value) ro.observe(canvasRef.value);
    if (stageRef.value) ro.observe(stageRef.value);
  });
});

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("resize", scheduleLayout);
  if (lineRaf) cancelAnimationFrame(lineRaf);
  window.clearTimeout(lineTimer);
  ro?.disconnect();
});

watch(
  () => [draft.value, selectedSlot.value, addMenuFor.value],
  () => nextTick(layoutStage),
  { deep: true },
);
</script>

<template>
  <component
    :is="page ? 'div' : AppDialog"
    v-bind="wrapperProps"
    @close="cancel()"
  >
    <div
      class="app-dialog-panel app-dialog-panel-soft flex w-full flex-col overflow-hidden"
      :class="[
        isDark ? 'text-stone-100' : 'text-stone-900',
        page
          ? 'h-full min-h-0 flex-1 rounded-none border-0'
          : 'h-[min(54rem,calc(100vh-2rem))] max-w-6xl rounded-3xl border',
      ]"
      :role="page ? undefined : 'dialog'"
      :aria-modal="page ? undefined : 'true'"
      :aria-label="t('view.settings')"
    >
      <p
        v-if="page"
        class="shrink-0 px-4 pt-3 text-xs leading-relaxed"
        :class="isDark ? 'text-stone-400' : 'text-stone-500'"
      >
        {{ t('view.settingsHint') }}
      </p>
      <header
        v-if="!page"
        class="flex shrink-0 items-start justify-between gap-4 border-b px-5 py-4 md:px-6"
        :class="isDark ? 'border-stone-800' : 'border-stone-100'"
      >
        <div class="min-w-0">
          <h3 class="text-base font-semibold tracking-tight">{{ t('view.settings') }}</h3>
          <p class="mt-1 text-xs leading-relaxed" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
            {{ t('view.settingsHint') }}
          </p>
        </div>
        <button
          class="rounded-xl p-2 transition"
          :class="isDark ? 'text-stone-400 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-500 hover:bg-stone-100 hover:text-stone-800'"
          :aria-label="t('albums.cancelBtn')"
          @click="cancel"
        >
          <IconClose class="h-5 w-5" />
        </button>
      </header>

      <div class="shrink-0 px-4 pb-3 pt-3 md:px-8">
        <div
          class="rounded-2xl border p-3.5 shadow-sm md:p-4"
          :class="isDark ? 'border-amber-900/60 bg-amber-950/25' : 'border-amber-200/80 bg-amber-50/75'"
        >
        <p class="mb-2 text-[11px] font-semibold uppercase tracking-wide" :class="isDark ? 'text-amber-300' : 'text-amber-800'">
          {{ t('view.burstSettings') }}
        </p>
        <label class="block text-xs font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
          {{ t('view.burstAlgorithm') }}
        </label>
        <select
          v-model="burstAlgorithmDraft"
          class="mt-2 w-full rounded-xl border px-3 py-2 text-sm"
          :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-200 bg-white text-stone-800'"
        >
          <option value="dct">{{ t('view.burstAlgorithmDct') }}</option>
          <option value="embedding">{{ t('view.burstAlgorithmEmbedding') }}</option>
        </select>

        <label class="mt-3 block text-xs font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
          {{ t('view.burstTimeWindow') }}
        </label>
        <select
          v-model.number="burstTimeWindowDraft"
          class="mt-2 w-full rounded-xl border px-3 py-2 text-sm"
          :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-200 bg-white text-stone-800'"
        >
          <option v-for="option in burstTimeWindowOptions" :key="option.value" :value="option.value">
            {{ option.label }}
          </option>
        </select>
        <p class="mt-1 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t('view.burstTimeWindowHint') }}
        </p>

        <div class="mt-3 flex items-center justify-between gap-4">
          <div class="min-w-0">
            <p class="text-xs font-semibold" :class="isDark ? 'text-stone-200' : 'text-stone-700'">
              {{ burstAlgorithmDraft === 'embedding' ? t('view.burstThresholdEmbedding') : t('view.burstThresholdDct') }}
            </p>
            <p class="mt-0.5 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
              {{ burstAlgorithmDraft === 'embedding' ? t('view.burstEmbeddingHint') : t('view.burstThresholdHint') }}
            </p>
          </div>
          <span class="shrink-0 text-sm font-semibold tabular-nums" :class="isDark ? 'text-amber-300' : 'text-amber-700'">
            {{ burstAlgorithmDraft === 'embedding' ? `${burstEmbeddingThresholdDraft}%` : burstThresholdDraft }}
          </span>
        </div>
        <input
          v-if="burstAlgorithmDraft === 'dct'"
          v-model.number="burstThresholdDraft"
          type="range"
          min="0"
          max="32"
          step="1"
          class="mt-3 w-full accent-amber-500"
          :aria-label="t('view.burstThresholdDct')"
        />
        <input
          v-else
          v-model.number="burstEmbeddingThresholdDraft"
          type="range"
          min="30"
          max="100"
          step="1"
          class="mt-3 w-full accent-amber-500"
          :aria-label="t('view.burstThresholdEmbedding')"
        />
        </div>
      </div>
      <div
        class="shrink-0 border-b px-4 py-3 md:px-8"
        :class="isDark ? 'border-stone-800 bg-stone-900/50' : 'border-stone-100 bg-stone-50/80'"
      >
        <p class="text-[11px] font-semibold uppercase tracking-wide" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t('view.unusedFields') }}
        </p>
        <div v-if="unused.length" class="mt-2 flex flex-wrap gap-1.5">
          <button
            v-for="field in unused"
            :key="field.key"
            type="button"
            class="rounded-full px-3.5 py-1.5 text-xs font-medium transition"
            :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-white text-stone-700 shadow-sm hover:bg-stone-100'"
            @click="pickUnused(field.key)"
          >
            {{ t(field.labelKey) }}
          </button>
        </div>
        <p v-else class="mt-2 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
          {{ t('view.noUnused') }}
        </p>
      </div>

      <div class="app-scrollbar min-h-0 flex-1 overflow-y-auto">
        <div ref="canvasRef" class="relative mx-auto w-full px-4 py-5 md:px-8 md:py-7">
          <svg
            v-if="!page"
            class="pointer-events-none absolute inset-0 z-10 hidden h-full w-full md:block"
            aria-hidden="true"
          >
            <path
              v-for="line in connectors"
              :key="line.id"
              :d="line.d"
              fill="none"
              :stroke="line.active
                ? (isDark ? '#d6d3d1' : '#57534e')
                : (isDark ? 'rgba(168, 162, 158, 0.55)' : 'rgba(120, 113, 108, 0.5)')"
              :stroke-width="line.active ? 1.8 : 1.35"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
            <circle
              v-for="line in connectors"
              :key="`${line.id}-from`"
              :cx="line.from.x"
              :cy="line.from.y"
              r="2.2"
              :fill="line.active
                ? (isDark ? '#d6d3d1' : '#57534e')
                : (isDark ? '#a8a29e' : '#78716c')"
            />
            <circle
              v-for="line in connectors"
              :key="`${line.id}-to`"
              :cx="line.to.x"
              :cy="line.to.y"
              r="2.2"
              :fill="line.active
                ? (isDark ? '#d6d3d1' : '#57534e')
                : (isDark ? '#a8a29e' : '#78716c')"
            />
          </svg>

          <div ref="stageRef" class="relative z-0 flex flex-col items-center gap-3 md:flex-row md:items-start md:justify-center md:gap-4">
            <div v-if="!page" class="relative hidden w-64 shrink-0 md:block" :style="{ height: `${sideHeight}px` }">
              <div
                v-for="item in leftSlots"
                :key="item.id"
                class="absolute inset-x-0"
                :data-slot-item="item.id"
                :style="{ top: `${pickerTop[item.id] || 0}px` }"
              >
                <div
                  class="relative w-full cursor-pointer rounded-2xl border px-3.5 py-3 text-left transition"
                  :class="pickerClass(item.id)"
                  @click="selectPosition(item.id)"
                >
                  <p class="text-xs font-semibold">{{ item.label }}</p>
                  <div class="mt-2 flex min-w-0 flex-wrap items-center gap-1">
                    <span
                      v-for="key in item.fields"
                      :key="key"
                      class="inline-flex max-w-full items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium"
                      :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-700 hover:bg-stone-200'"
                      @click.stop="dropField(key)"
                    >
                      <span class="truncate">{{ fieldLabel(key) }}</span>
                      <span class="opacity-50">×</span>
                    </span>
                    <span
                      v-if="unused.length > 0"
                      class="inline-flex items-center gap-1 rounded-full border border-dashed px-2.5 py-1.5 text-xs"
                      :class="isDark ? 'border-stone-600 text-stone-400 hover:border-stone-400 hover:text-stone-200' : 'border-stone-300 text-stone-500 hover:border-stone-500 hover:text-stone-700'"
                      @click.stop="toggleAddMenu(item.id)"
                    >
                      <IconPlus class="h-3.5 w-3.5" />
                      {{ t('view.addField') }}
                    </span>
                    <span
                      v-else-if="item.fields.length === 0"
                      class="text-xs"
                      :class="isDark ? 'text-stone-500' : 'text-stone-400'"
                    >{{ t('view.emptySlot') }}</span>
                  </div>
                  <div
                    v-if="addMenuFor === item.id"
                    class="mt-2 flex flex-wrap gap-1 rounded-xl p-1.5"
                    :class="isDark ? 'bg-stone-950/80' : 'bg-stone-50'"
                    @click.stop
                  >
                    <button
                      v-for="field in unused"
                      :key="field.key"
                      type="button"
                      class="rounded-full px-2.5 py-1.5 text-xs font-medium"
                      :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-800 hover:bg-stone-200'"
                      @click="placeField(item.id, field.key)"
                    >
                      {{ t(field.labelKey) }}
                    </button>
                  </div>
                </div>
              </div>
            </div>

            <div class="w-[15.5rem] shrink-0">
              <PhotoCard
                :photo="PREVIEW_PHOTO"
                :layout="layout"
                :preview="true"
                :is-dark="isDark"
                :format-size="formatSize"
                :highlight-slot="selectedSlot"
                @select-slot="selectPosition"
              />
            </div>

            <div v-if="!page" class="relative hidden w-64 shrink-0 md:block" :style="{ height: `${sideHeight}px` }">
              <div
                v-for="item in rightSlots"
                :key="item.id"
                class="absolute inset-x-0"
                :data-slot-item="item.id"
                :style="{ top: `${pickerTop[item.id] || 0}px` }"
              >
                <div
                  class="relative w-full cursor-pointer rounded-2xl border px-3.5 py-3 text-left transition"
                  :class="pickerClass(item.id)"
                  @click="selectPosition(item.id)"
                >
                  <p class="text-xs font-semibold">{{ item.label }}</p>
                  <div class="mt-2 flex min-w-0 flex-wrap items-center gap-1">
                    <span
                      v-for="key in item.fields"
                      :key="key"
                      class="inline-flex max-w-full items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium"
                      :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-700 hover:bg-stone-200'"
                      @click.stop="dropField(key)"
                    >
                      <span class="truncate">{{ fieldLabel(key) }}</span>
                      <span class="opacity-50">×</span>
                    </span>
                    <span
                      v-if="unused.length > 0"
                      class="inline-flex items-center gap-1 rounded-full border border-dashed px-2.5 py-1.5 text-xs"
                      :class="isDark ? 'border-stone-600 text-stone-400 hover:border-stone-400 hover:text-stone-200' : 'border-stone-300 text-stone-500 hover:border-stone-500 hover:text-stone-700'"
                      @click.stop="toggleAddMenu(item.id)"
                    >
                      <IconPlus class="h-3.5 w-3.5" />
                      {{ t('view.addField') }}
                    </span>
                    <span
                      v-else-if="item.fields.length === 0"
                      class="text-xs"
                      :class="isDark ? 'text-stone-500' : 'text-stone-400'"
                    >{{ t('view.emptySlot') }}</span>
                  </div>
                  <div
                    v-if="addMenuFor === item.id"
                    class="mt-2 flex flex-wrap gap-1 rounded-xl p-1.5"
                    :class="isDark ? 'bg-stone-950/80' : 'bg-stone-50'"
                    @click.stop
                  >
                    <button
                      v-for="field in unused"
                      :key="field.key"
                      type="button"
                      class="rounded-full px-2.5 py-1.5 text-xs font-medium"
                      :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-800 hover:bg-stone-200'"
                      @click="placeField(item.id, field.key)"
                    >
                      {{ t(field.labelKey) }}
                    </button>
                  </div>
                </div>
              </div>
            </div>

            <div class="flex w-full max-w-sm flex-col gap-2" :class="page ? '' : 'md:hidden'">
              <div
                v-for="item in aroundSlots"
                :key="`m-${item.id}`"
                class="w-full cursor-pointer rounded-2xl border px-3.5 py-3 text-left transition"
                :class="pickerClass(item.id)"
                @click="selectPosition(item.id)"
              >
                <p class="text-xs font-semibold">{{ item.label }}</p>
                <div class="mt-2 flex min-w-0 flex-wrap items-center gap-1.5">
                  <span
                    v-for="key in item.fields"
                    :key="key"
                    class="inline-flex max-w-full items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium"
                    :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-700 hover:bg-stone-200'"
                    @click.stop="dropField(key)"
                  >
                    <span class="truncate">{{ fieldLabel(key) }}</span>
                    <span class="opacity-50">×</span>
                  </span>
                  <span
                    v-if="unused.length > 0"
                    class="inline-flex items-center gap-1 rounded-full border border-dashed px-2.5 py-1.5 text-xs"
                    :class="isDark ? 'border-stone-600 text-stone-400' : 'border-stone-300 text-stone-500'"
                    @click.stop="toggleAddMenu(item.id)"
                  >
                    <IconPlus class="h-3.5 w-3.5" />
                    {{ t('view.addField') }}
                  </span>
                </div>
              </div>
            </div>
          </div>

          <div class="relative z-0 mx-auto mt-4 flex w-64 flex-col gap-2.5">
            <div
              v-for="item in rowItems"
              :key="item.id"
              class="relative cursor-pointer rounded-2xl border px-3.5 py-3 transition"
              :class="pickerClass(item.id)"
              @click="selectPosition(item.id)"
            >
              <div class="flex items-start gap-2">
                <div class="min-w-0 flex-1">
                  <p class="text-xs font-semibold">{{ item.label }}</p>
                  <div class="mt-1.5 flex min-w-0 flex-wrap items-center gap-1">
                    <button
                      v-for="key in item.fields"
                      :key="key"
                      type="button"
                      class="inline-flex max-w-full items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium"
                      :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-700 hover:bg-stone-200'"
                      @click.stop="dropField(key)"
                    >
                      <span class="truncate">{{ fieldLabel(key) }}</span>
                      <span class="opacity-50">×</span>
                    </button>
                    <button
                      v-if="unused.length > 0"
                      type="button"
                      class="inline-flex items-center gap-1 rounded-full border border-dashed px-2.5 py-1.5 text-xs"
                      :class="isDark ? 'border-stone-600 text-stone-400 hover:border-stone-400 hover:text-stone-200' : 'border-stone-300 text-stone-500 hover:border-stone-500 hover:text-stone-700'"
                      @click.stop="toggleAddMenu(item.id)"
                    >
                      <IconPlus class="h-3.5 w-3.5" />
                      {{ t('view.addField') }}
                    </button>
                    <span
                      v-else-if="item.fields.length === 0"
                      class="text-xs"
                      :class="isDark ? 'text-stone-500' : 'text-stone-400'"
                    >{{ t('view.emptySlot') }}</span>
                  </div>
                  <div
                    v-if="addMenuFor === item.id"
                    class="mt-2 flex flex-wrap gap-1 rounded-xl p-1.5"
                    :class="isDark ? 'bg-stone-950/80' : 'bg-stone-50'"
                    @click.stop
                  >
                    <button
                      v-for="field in unused"
                      :key="field.key"
                      type="button"
                      class="rounded-full px-2.5 py-1.5 text-xs font-medium"
                      :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-100 text-stone-800 hover:bg-stone-200'"
                      @click="placeField(item.id, field.key)"
                    >
                      {{ t(field.labelKey) }}
                    </button>
                  </div>
                </div>
                <button
                  type="button"
                  class="shrink-0 rounded-lg p-1.5 transition"
                  :class="isDark ? 'text-stone-500 hover:bg-stone-800 hover:text-rose-300' : 'text-stone-400 hover:bg-stone-100 hover:text-rose-600'"
                  :title="t('view.removeRow')"
                  @click.stop="removeRow(item.index)"
                >
                  <IconClose class="h-4 w-4" />
                </button>
              </div>
            </div>

            <button
              v-if="(layout.rows || []).length < MAX_BELOW_ROWS"
              type="button"
              class="inline-flex items-center justify-center gap-1.5 rounded-2xl border border-dashed px-3 py-2.5 text-sm font-medium transition"
              :class="isDark ? 'border-stone-700 text-stone-300 hover:border-stone-500 hover:text-stone-100' : 'border-stone-300 text-stone-600 hover:border-stone-400 hover:text-stone-800'"
              @click="addRow"
            >
              <IconPlus class="h-4 w-4" />
              {{ t('view.addRow') }}
            </button>
          </div>
        </div>
      </div>

      <footer
        class="flex shrink-0 items-center justify-between gap-3 border-t px-5 py-3 md:px-6"
        :class="[
          isDark ? 'border-stone-800' : 'border-stone-100',
          page ? 'pb-[max(0.75rem,var(--safe-bottom,0px))]' : '',
        ]"
      >
        <button
          class="inline-flex items-center gap-1.5 rounded-xl px-3.5 py-2.5 text-sm font-medium transition"
          :class="isDark ? 'text-stone-300 hover:bg-stone-800' : 'text-stone-600 hover:bg-stone-100'"
          @click="resetLayout"
        >
          <IconRefresh class="h-3.5 w-3.5" />
          {{ t('view.reset') }}
        </button>
        <div class="flex items-center gap-2">
          <button
            class="rounded-xl px-4 py-2 text-sm font-medium transition"
            :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
            @click="cancel"
          >
            {{ t('view.cancel') }}
          </button>
          <button
            class="rounded-xl px-4 py-2 text-sm font-medium transition"
            :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
            @click="confirm"
          >
            {{ t('view.done') }}
          </button>
        </div>
      </footer>
    </div>
  </component>
</template>
