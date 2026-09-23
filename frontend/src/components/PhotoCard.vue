<script setup>
import { computed, nextTick, ref, watch } from "vue";
import { IconCheck, IconStar } from "../utils/icons";
import { t } from "../i18n/index.js";
import LazyImage from "./LazyImage.vue";
import {
  COLOR_LABELS,
  OVERLAY_SLOTS,
  normalizeColorLabels,
} from "../utils/photoCardLayout.js";

const props = defineProps({
  photo: { type: Object, required: true },
  layout: { type: Object, required: true },
  compact: { type: Boolean, default: false },
  hideOverlays: { type: Boolean, default: false },
  preview: { type: Boolean, default: false },
  isDark: { type: Boolean, default: false },
  thumbSrc: { type: String, default: "" },
  formatSize: { type: Function, default: null },
  selectMode: { type: Boolean, default: false },
  selected: { type: Boolean, default: false },
  highlightSlot: { type: String, default: "" },
});

const emit = defineEmits(["click", "select-slot"]);

function onSlotClick(id) {
  if (!props.preview) return;
  emit("select-slot", id);
}

const OVERLAY_POS = {
  tl: "top-1.5 left-1.5 items-start justify-start",
  tr: "top-1.5 right-1.5 items-start justify-end",
  bl: "bottom-1.5 left-1.5 items-end justify-start",
  br: "bottom-1.5 right-1.5 items-end justify-end",
};

const thumbsOnly = computed(() => !props.preview && props.compact);
const hideOverlays = computed(() => !props.preview && props.hideOverlays);
const animateBelow = ref(true);
const imageBoxRef = ref(null);
const cardMotion = "[contain:content] overflow-hidden transition-[border-radius,border-color,background-color,box-shadow,padding] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]";

let imageZoomGen = 0;

function clearImageZoom(el) {
  el.style.transition = "";
  el.style.transform = "";
  el.style.transformOrigin = "";
}

async function playImageZoom() {
  const box = imageBoxRef.value;
  if (!box) return;
  const first = box.getBoundingClientRect();
  const gen = ++imageZoomGen;
  await nextTick();
  if (gen !== imageZoomGen) return;
  const last = box.getBoundingClientRect();
  if (!last.width || !last.height) return;
  const sx = first.width / last.width;
  if (!Number.isFinite(sx) || Math.abs(sx - 1) < 0.008) return;
  const media = box.querySelector("img") || box.firstElementChild;
  if (!media) return;
  media.style.transition = "none";
  media.style.transformOrigin = "center center";
  media.style.transform = `scale(${sx})`;
  box.getBoundingClientRect();
  requestAnimationFrame(() => {
    if (gen !== imageZoomGen) return;
    requestAnimationFrame(() => {
      if (gen !== imageZoomGen) return;
      media.style.transition = "transform 300ms cubic-bezier(0.22, 1, 0.36, 1)";
      media.style.transform = "none";
      const done = (event) => {
        if (event?.propertyName && event.propertyName !== "transform") return;
        media.removeEventListener("transitionend", done);
        if (gen !== imageZoomGen) return;
        clearImageZoom(media);
      };
      media.addEventListener("transitionend", done);
      setTimeout(() => {
        if (gen !== imageZoomGen) return;
        media.removeEventListener("transitionend", done);
        if (media.style.transition) clearImageZoom(media);
      }, 380);
    });
  });
}

watch(
  () => [thumbsOnly.value, props.layout?.thumbnailSize],
  ([only, size], prev) => {
    if (props.preview) return;
    const [wasOnly, wasSize] = prev || [];
    if (wasOnly === undefined) return;
    const compactChanged = only !== wasOnly;
    const sizeChanged = size !== wasSize;
    animateBelow.value = compactChanged && !sizeChanged;
    if (compactChanged && !sizeChanged) playImageZoom();
    else imageZoomGen += 1;
  },
  { flush: "pre" },
);

const cardClass = computed(() => {
  if (thumbsOnly.value) {
    return [
      "group self-start cursor-pointer rounded-none border border-transparent bg-transparent p-0",
      cardMotion,
      props.selected ? "ring-2 ring-sky-400" : "",
    ];
  }
  return [
    "group self-start cursor-pointer rounded-xl p-1.5 sm:p-2",
    cardMotion,
    props.selected
      ? (props.isDark ? "border border-sky-500 bg-sky-900/30 ring-1 ring-sky-500/50" : "border border-sky-400 bg-sky-50 ring-1 ring-sky-400/50")
      : "app-surface app-card",
  ];
});

function colorDot(label) {
  return COLOR_LABELS.find((item) => item.key === label);
}

function fieldHasValue(key) {
  const photo = props.photo;
  if (key === "rating") return Number(photo.rating) > 0;
  if (key === "tags") return Array.isArray(photo.tags) && photo.tags.length > 0;
  if (key === "color") return normalizeColorLabels(photo.color_label).length > 0;
  if (key === "size") return Number(photo.size || photo.size_bytes) > 0;
  if (key === "name") return true;
  if (key === "datetime") return !!(photo.datetime || photo.date);
  if (key === "camera") return !!photo.camera_model;
  if (key === "lens") return !!photo.lens;
  return false;
}

function visibleFields(keys) {
  const list = Array.isArray(keys) ? keys : [];
  if (props.preview) return list;
  return list.filter((key) => fieldHasValue(key));
}

function visibleRowFields(row) {
  return visibleFields(row);
}

const visibleRows = computed(() =>
  (props.layout.rows || [])
    .map((fields, index) => ({ fields: visibleRowFields(fields), index }))
    .filter((row) => props.preview || row.fields.length > 0),
);

const hasBelowRows = computed(() => props.preview || visibleRows.value.length > 0);
const expandBelow = computed(() => {
  if (props.preview) return true;
  if (props.compact) return false;
  return visibleRows.value.length > 0;
});

function slotFields(slotId) {
  return visibleFields(props.layout.slots?.[slotId] || []);
}

function displayText(key) {
  const photo = props.photo;
  if (key === "name") return photo.name || "";
  if (key === "datetime") return photo.datetime || photo.date || "";
  if (key === "camera") return photo.camera_model || "";
  if (key === "lens") return photo.lens || "";
  if (key === "size") {
    const bytes = Number(photo.size || photo.size_bytes) || 0;
    return props.formatSize ? props.formatSize(bytes) : `${bytes}`;
  }
  return "";
}
</script>

<template>
  <article
    :class="cardClass"
    :data-preview-root="preview || undefined"
    :data-flip-id="photo.path || undefined"
    @click="emit('click')"
  >
    <div
      ref="imageBoxRef"
      data-photo-image
      class="relative aspect-square overflow-hidden transition-[border-radius] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]"
      :data-preview-image="preview || undefined"
      :class="[
        thumbsOnly ? 'rounded-none' : 'rounded-lg',
        isDark ? 'bg-stone-800' : 'bg-stone-100',
      ]"
    >
      <LazyImage
        v-if="thumbSrc"
        :img-class="[
          'absolute inset-0 h-full w-full object-cover',
          thumbsOnly ? '' : 'transition group-hover:scale-105',
        ].join(' ')"
        :src="thumbSrc"
        alt=""
      />
      <div
        v-else
        class="absolute inset-0"
        :class="isDark
          ? 'bg-gradient-to-br from-stone-700 via-stone-800 to-amber-950/70'
          : 'bg-gradient-to-br from-stone-200 via-amber-50 to-sky-100'"
      >
        <svg class="absolute inset-0 h-full w-full opacity-70" viewBox="0 0 200 200" preserveAspectRatio="xMidYMid slice" aria-hidden="true">
          <circle :cx="148" :cy="46" r="18" fill="var(--color-amber-400)" opacity="0.85" />
          <path d="M0 138 L52 92 L86 118 L128 70 L200 132 V200 H0 Z" :fill="isDark ? '#44403c' : '#a8a29e'" />
          <path d="M0 162 L70 118 L110 148 L200 108 V200 H0 Z" :fill="isDark ? '#292524' : '#78716c'" opacity="0.85" />
        </svg>
      </div>

      <div
        v-if="selectMode"
        data-photo-overlay
        class="absolute right-1.5 top-1.5 z-20 flex h-5 w-5 items-center justify-center rounded-full border-2 transition"
        :class="selected
          ? 'border-sky-400 bg-sky-500 text-white'
          : (isDark ? 'border-stone-400 bg-stone-900/60' : 'border-stone-400 bg-white/80')"
      >
        <IconCheck v-if="selected" class="h-3 w-3" />
      </div>

      <div
        v-for="slot in OVERLAY_SLOTS"
        :key="slot.id"
        data-photo-overlay
        class="absolute z-10 flex max-w-[88%] flex-nowrap items-center gap-0.5 overflow-hidden rounded transition-opacity duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]"
        :class="[
          OVERLAY_POS[slot.corner],
          hideOverlays ? 'pointer-events-none opacity-0' : 'opacity-100',
          preview ? 'cursor-pointer p-0.5 hover:ring-1 hover:ring-sky-400/70' : '',
          preview && highlightSlot === slot.id ? 'ring-1 ring-sky-400 ring-offset-1 ring-offset-transparent' : '',
        ]"
        :data-preview-slot="preview ? slot.id : undefined"
        @click.stop="onSlotClick(slot.id)"
      >
        <template v-if="slotFields(slot.id).length">
          <template v-for="key in slotFields(slot.id)" :key="key">
            <span v-if="key === 'color'" class="inline-flex shrink-0 items-center gap-0.5">
              <span
                v-for="clKey in normalizeColorLabels(photo.color_label)"
                :key="clKey"
                class="h-2.5 w-2.5 rounded-full"
                :class="colorDot(clKey)?.color || 'bg-stone-400'"
              />
            </span>
            <span v-else-if="key === 'rating'" class="inline-flex shrink-0">
              <IconStar v-for="s in photo.rating" :key="s" class="h-3 w-3 text-yellow-400 drop-shadow" />
            </span>
            <span
              v-else-if="key === 'tags'"
              class="flex shrink-0 items-center gap-0.5"
            >
              <span
                v-for="tag in (photo.tags || []).slice(0, 2)"
                :key="tag"
                class="rounded bg-black/45 px-1 py-px text-[9px] text-white"
              >{{ tag }}</span>
            </span>
            <span
              v-else
              class="min-w-0 truncate rounded bg-black/45 px-1 py-px text-[10px] text-white drop-shadow"
            >{{ displayText(key) }}</span>
          </template>
        </template>
        <span
          v-else-if="preview"
          class="h-5 min-w-5 rounded border border-dashed opacity-50"
          :class="isDark ? 'border-amber-200/50 bg-stone-950/20' : 'border-amber-700/35 bg-white/30'"
        />
      </div>
    </div>

    <div
      v-if="hasBelowRows"
      data-photo-below
      class="grid min-w-0"
      :class="[
        animateBelow ? 'transition-[grid-template-rows,opacity] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)]' : '',
        expandBelow ? 'opacity-100' : 'pointer-events-none opacity-0',
      ]"
      :style="{ gridTemplateRows: expandBelow ? '1fr' : '0fr' }"
    >
    <div class="min-h-0 overflow-hidden leading-tight">
      <div
        v-for="(row, rowIndex) in visibleRows"
        :key="row.index"
        class="flex min-w-0 items-baseline overflow-hidden whitespace-nowrap rounded"
        :class="[
          rowIndex === 0 ? 'mt-1.5' : '',
          rowIndex > 0 && row.fields.length === 1 && row.fields[0] === 'tags' ? 'mt-0.5' : '',
          preview ? 'cursor-pointer px-0.5 hover:ring-1 hover:ring-sky-400/70' : '',
          preview && highlightSlot === `row-${row.index}` ? 'ring-1 ring-sky-400' : '',
        ]"
        :data-preview-slot="preview ? `row-${row.index}` : undefined"
        @click.stop="onSlotClick(`row-${row.index}`)"
      >
        <template v-if="row.fields.length">
          <template v-for="(key, fieldIndex) in row.fields" :key="key">
            <span
              v-if="fieldIndex > 0"
              class="mx-1 shrink-0 text-[10px] opacity-40"
            >·</span>
            <span
              v-if="key === 'color'"
              class="inline-flex shrink-0 items-center gap-0.5 align-middle"
            >
              <span
                v-for="clKey in normalizeColorLabels(photo.color_label)"
                :key="clKey"
                class="h-2 w-2 rounded-full"
                :class="colorDot(clKey)?.color || 'bg-stone-400'"
              />
            </span>
            <span v-else-if="key === 'rating'" class="inline-flex shrink-0 align-middle">
              <IconStar v-for="s in photo.rating" :key="s" class="h-3 w-3 text-yellow-400" />
            </span>
            <span
              v-else-if="key === 'tags'"
              class="flex shrink-0 items-center gap-0.5"
            >
              <span
                v-for="tag in (photo.tags || []).slice(0, 3)"
                :key="tag"
                class="rounded px-1 py-0 text-[9px]"
                :class="isDark ? 'bg-sky-900/50 text-sky-200' : 'bg-sky-50 text-sky-600'"
              >{{ tag }}</span>
              <span
                v-if="(photo.tags || []).length > 3"
                class="shrink-0 text-[9px]"
                :class="isDark ? 'text-stone-500' : 'text-stone-400'"
              >+{{ photo.tags.length - 3 }}</span>
            </span>
            <span
              v-else
              class="min-w-0 truncate"
              :class="key === 'name'
                ? 'text-xs font-semibold'
                : (isDark ? 'text-[10px] text-stone-400' : 'text-[10px] text-stone-500')"
              :title="displayText(key)"
            >{{ displayText(key) }}</span>
          </template>
        </template>
        <span
          v-else-if="preview"
          class="block h-4 w-full rounded border border-dashed opacity-40"
          :class="isDark ? 'border-amber-200/40' : 'border-amber-800/25'"
        />
      </div>
      <p v-if="preview && (!layout.rows || layout.rows.length === 0)" class="mt-1.5 text-[10px] italic opacity-40">
        {{ t('view.noBelowRows') }}
      </p>
    </div>
    </div>
  </article>
</template>
