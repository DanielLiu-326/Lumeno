<script setup>
/**
 * LazyImage — LIFO-stack-backed throttled loader with no-flicker remount.
 *
 * Flow for a new image:
 *   1. Component mounts (virtua puts the row in the DOM).
 *   2. IntersectionObserver fires when the <img> element is truly visible
 *      inside the scroll container (auto-detected as the nearest overflow
 *      scroll ancestor — i.e. VList's container).
 *   3. enqueue() pushes the URL onto the LIFO stack.
 *   4. A worker pops it and preloads via a hidden Image element.
 *   5. On success, the URL is added to loadedUrls and displaySrc is set.
 *   6. The visible <img> renders using the browser's memory cache — instant.
 *
 * On remount (virtua scrolls back): loadedUrls hit → displaySrc set during
 * setup() → first render already has src, zero blank frames.
 */
import { ref, watch, onMounted, onBeforeUnmount } from "vue";
import { enqueue, loadedUrls } from "../utils/imageThrottle.js";

const props = defineProps({
  src: { type: String, required: true },
  imgClass: { type: String, default: "" },
  alt: { type: String, default: "" },
});

const imgRef = ref(null);

// Initialise from cache so the very first render already has src set.
const displaySrc = ref(loadedUrls.has(props.src) ? props.src : null);
const failed = ref(false);

let alive = true;
let controller = null;
let observer = null;

// ── Helpers ────────────────────────────────────────────────────────────────

function cancelPending() {
  if (controller) { controller.abort(); controller = null; }
}

function disconnectObserver() {
  if (observer) { observer.disconnect(); observer = null; }
}

/**
 * Walk up the DOM tree to find the nearest overflow-scroll ancestor.
 * For images inside VirtualGrid this returns VList's scroll container,
 * which gives IntersectionObserver the correct root.
 */
function findScrollRoot(el) {
  let node = el?.parentElement;
  while (node && node !== document.documentElement) {
    const { overflow, overflowY } = getComputedStyle(node);
    if (/auto|scroll/.test(overflow) || /auto|scroll/.test(overflowY)) return node;
    node = node.parentElement;
  }
  return null; // falls back to browser viewport
}

// ── Load logic ─────────────────────────────────────────────────────────────

async function startLoad() {
  const url = props.src;

  // JS cache hit — set src immediately; browser serves from memory cache.
  if (loadedUrls.has(url)) {
    displaySrc.value = url;
    failed.value = false;
    return;
  }

  cancelPending();
  displaySrc.value = null;
  failed.value = false;

  controller = new AbortController();
  try {
    await enqueue(url, controller.signal);
  } catch {
    return; // aborted or preload error
  }
  controller = null;

  if (!alive || props.src !== url) return;
  // Worker added url to loadedUrls; render the visible <img> from cache.
  displaySrc.value = url;
}

/**
 * Observe the <img> element; call startLoad() the moment it intersects
 * the scroll container's viewport (+ 200px lookahead margin).
 */
function observeAndLoad() {
  disconnectObserver();
  if (!imgRef.value) return;

  const root = findScrollRoot(imgRef.value);
  observer = new IntersectionObserver(
    ([entry]) => {
      if (entry.isIntersecting) {
        disconnectObserver();
        startLoad();
      }
    },
    { root, rootMargin: "200px" },
  );
  observer.observe(imgRef.value);
}

// ── Event handlers ─────────────────────────────────────────────────────────

function onError() {
  if (!alive) return;
  // Rare: browser couldn't serve a URL that was in loadedUrls (cache evicted).
  if (displaySrc.value) loadedUrls.delete(displaySrc.value);
  displaySrc.value = null;
  failed.value = true;
}

function onImageClick() {
  if (failed.value) {
    loadedUrls.delete(props.src);
    startLoad();
  }
}

// ── Lifecycle ──────────────────────────────────────────────────────────────

onMounted(() => {
  alive = true;
  if (displaySrc.value) return; // resolved from cache, nothing to do
  observeAndLoad();
});

onBeforeUnmount(() => {
  alive = false;
  disconnectObserver();
  cancelPending();
  displaySrc.value = null;
});

watch(
  () => props.src,
  (newSrc) => {
    disconnectObserver();
    cancelPending();
    failed.value = false;
    if (loadedUrls.has(newSrc)) {
      displaySrc.value = newSrc;
      return;
    }
    displaySrc.value = null;
    if (alive) observeAndLoad();
  },
);
</script>

<template>
  <img
    ref="imgRef"
    v-bind="displaySrc ? { src: displaySrc } : {}"
    :alt="alt"
    :class="imgClass"
    decoding="async"
    fetchpriority="low"
    @error="onError"
    @click="onImageClick"
  />
</template>
