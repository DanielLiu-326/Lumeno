<script setup>
/** Lazy-loaded face avatar via album face-crop API. */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { personFaceCropUrl } from "../utils/faceCrop.js";

const props = defineProps({
  /** Album name or shared `owner/album` for face-crop API. */
  album: { type: String, default: "" },
  cover: { type: Object, default: null },
  faceId: { type: String, default: "" },
  /** Album-relative source when face_id is unavailable. */
  source: { type: String, default: "" },
  faceBox: { type: Array, default: null },
  /** CSS display size in pixels (does not affect API request size). */
  size: { type: Number, default: 128 },
  imgClass: { type: String, default: "" },
  lazy: { type: Boolean, default: true },
});

const root = ref(null);
const visible = ref(!props.lazy);
const loadFailed = ref(false);
const retryNonce = ref(0);
let observer = null;

const cropSubject = computed(() => {
  if (props.cover) return props.cover;
  if (props.faceId || props.source) {
    return {
      album: props.album,
      face_id: props.faceId,
      source: props.source,
      face_box: props.faceBox,
    };
  }
  return null;
});

const baseSrc = computed(() => {
  if (!visible.value || !cropSubject.value) return "";
  return personFaceCropUrl(cropSubject.value, props.album);
});

const src = computed(() => {
  const url = baseSrc.value;
  if (!url || !retryNonce.value) return url;
  const sep = url.includes("?") ? "&" : "?";
  return `${url}${sep}_=${retryNonce.value}`;
});

function observe() {
  if (!props.lazy || !root.value || typeof IntersectionObserver === "undefined") {
    visible.value = true;
    return;
  }
  observer?.disconnect();
  observer = new IntersectionObserver(
    (entries) => {
      if (entries.some((e) => e.isIntersecting)) {
        visible.value = true;
        observer?.disconnect();
        observer = null;
      }
    },
    { rootMargin: "120px", threshold: 0.01 },
  );
  observer.observe(root.value);
}

function onError() {
  if (retryNonce.value === 0) {
    retryNonce.value = Date.now();
    loadFailed.value = false;
    return;
  }
  loadFailed.value = true;
}

function resetLoadState() {
  loadFailed.value = false;
  retryNonce.value = 0;
}

onMounted(observe);
watch(
  () => [props.album, props.faceId, props.source, props.faceBox?.join(","), props.cover],
  () => {
    visible.value = !props.lazy;
    resetLoadState();
    observe();
  },
);
onBeforeUnmount(() => {
  observer?.disconnect();
  observer = null;
});
</script>

<template>
  <div ref="root" class="relative h-full w-full overflow-hidden">
    <img
      v-if="src && !loadFailed"
      :src="src"
      alt=""
      :class="['h-full w-full object-cover', imgClass]"
      loading="lazy"
      decoding="async"
      @error="onError"
    />
  </div>
</template>
