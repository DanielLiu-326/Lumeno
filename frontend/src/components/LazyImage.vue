<script setup>
/**
 * LazyImage — LIFO-stack-backed throttled loader with no-flicker remount.
 *
 * Flow for a new image:
 *   1. Component mounts (virtua puts the row in the DOM).
 *   2. enqueue() starts immediately on a background Image() — no extra delay,
 *      and it still runs if the real grid is visibility:hidden during zoom.
 *   3. On success, displaySrc is set from the memory cache.
 *   4. Cached remounts set src during setup() so the first paint already has it.
 *   5. First-time reveals fade in; cache hits appear immediately.
 */
import { ref, watch, onMounted, onBeforeUnmount, nextTick } from "vue";
import { enqueue, loadedUrls } from "../utils/imageThrottle.js";

const props = defineProps({
  src: { type: String, required: true },
  imgClass: { type: String, default: "" },
  imgStyle: { type: [Object, String], default: null },
  alt: { type: String, default: "" },
});

const imgRef = ref(null);

const displaySrc = ref(loadedUrls.has(props.src) ? props.src : null);
const appear = ref(loadedUrls.has(props.src));
const fadeIn = ref(false);
const failed = ref(false);

let alive = true;
let controller = null;

function cancelPending() {
  if (controller) { controller.abort(); controller = null; }
}

async function startLoad() {
  const url = props.src;

  if (loadedUrls.has(url)) {
    displaySrc.value = url;
    appear.value = true;
    fadeIn.value = false;
    failed.value = false;
    return;
  }

  cancelPending();
  displaySrc.value = null;
  appear.value = false;
  fadeIn.value = false;
  failed.value = false;

  controller = new AbortController();
  try {
    await enqueue(url, controller.signal, { priority: "auto" });
  } catch {
    return;
  }
  controller = null;

  if (!alive || props.src !== url) return;
  displaySrc.value = url;
  await nextTick();
  if (!alive || props.src !== url) return;
  try {
    if (imgRef.value?.decode) {
      await Promise.race([
        imgRef.value.decode(),
        new Promise((resolve) => setTimeout(resolve, 48)),
      ]);
    }
  } catch {
    /* decode can reject on abort / broken image */
  }
  if (!alive || props.src !== url) return;
  requestAnimationFrame(() => {
    if (alive && props.src === url) fadeIn.value = true;
  });
}

function onError() {
  if (!alive) return;
  if (displaySrc.value) loadedUrls.delete(displaySrc.value);
  displaySrc.value = null;
  appear.value = false;
  fadeIn.value = false;
  failed.value = true;
}

function onImageClick() {
  if (failed.value) {
    loadedUrls.delete(props.src);
    startLoad();
  }
}

onMounted(() => {
  alive = true;
  if (displaySrc.value) return;
  startLoad();
});

onBeforeUnmount(() => {
  alive = false;
  cancelPending();
  displaySrc.value = null;
});

watch(
  () => props.src,
  (newSrc) => {
    cancelPending();
    failed.value = false;
    if (loadedUrls.has(newSrc)) {
      displaySrc.value = newSrc;
      appear.value = true;
      fadeIn.value = false;
      return;
    }
    displaySrc.value = null;
    appear.value = false;
    fadeIn.value = false;
    if (alive) startLoad();
  },
);
</script>

<template>
  <img
    ref="imgRef"
    v-bind="displaySrc ? { src: displaySrc } : {}"
    :alt="alt"
    :class="[imgClass, displaySrc && !appear && !fadeIn ? 'opacity-0' : '', fadeIn ? 'lazy-image-in' : '']"
    :style="imgStyle || undefined"
    decoding="async"
    draggable="false"
    fetchpriority="auto"
    @error="onError"
    @click="onImageClick"
  />
</template>

<style scoped>
.lazy-image-in {
  animation: lazy-image-in 180ms ease-out both;
}

@keyframes lazy-image-in {
  from { opacity: 0; }
  to { opacity: 1; }
}
</style>
