// Thumbnail grid appearance: the stored view layout, the size that follows the
// zoom gesture, the column/gap classes derived from that size, and the small
// popover + toast the size control uses.
//
// Kept out of AlbumPanel so the panel keeps rendering while this file keeps the
// layout math. The panel still owns the gesture handlers; they only need
// `photoGridRef` and `zoomChromeLock` from here.
import { computed, nextTick, ref, watch } from "vue";

import {
  THUMB_DETAILS_MIN,
  THUMB_MAX,
  hasBelowInfo,
  loadViewLayout,
  saveViewLayout,
} from "../utils/photoCardLayout.js";

const THUMB_GRID_COLS = [
  "grid-cols-1 sm:grid-cols-1 lg:grid-cols-2 xl:grid-cols-2 2xl:grid-cols-2",
  "grid-cols-1 sm:grid-cols-2 lg:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-3",
  "grid-cols-2 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5",
  "grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-7",
  "grid-cols-3 sm:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-8",
  "grid-cols-4 sm:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8 2xl:grid-cols-10",
  "grid-cols-5 sm:grid-cols-6 lg:grid-cols-8 xl:grid-cols-10 2xl:grid-cols-12",
  "grid-cols-6 sm:grid-cols-7 lg:grid-cols-10 xl:grid-cols-12 2xl:grid-cols-14",
  "grid-cols-7 sm:grid-cols-8 lg:grid-cols-12 xl:grid-cols-14 2xl:grid-cols-16",
  "grid-cols-8 sm:grid-cols-10 lg:grid-cols-14 xl:grid-cols-16 2xl:grid-cols-18",
];
const TW_BREAKPOINTS = { sm: 640, md: 768, lg: 1024, xl: 1280, "2xl": 1536 };
const THUMB_GAP_PX = [4, 6, 6, 8, 8, 8, 8, 10, 10, 10];

function parseGridCols(spec, width = window.innerWidth) {
  let cols = 1;
  const re = /(?:^|\s)(?:(sm|md|lg|xl|2xl):)?grid-cols-(\d+)/g;
  let match = re.exec(spec);
  while (match) {
    const bp = match[1];
    const count = Number(match[2]);
    if (!bp || width >= (TW_BREAKPOINTS[bp] || 0)) cols = count;
    match = re.exec(spec);
  }
  return Math.max(1, cols);
}

function thumbColsForSize(size) {
  const spec = THUMB_GRID_COLS[THUMB_MAX - size] || THUMB_GRID_COLS[3];
  return parseGridCols(spec);
}

/// Next size that actually changes the column count, so a zoom step is visible.
function nextDistinctThumbSize(from, dir) {
  const startCols = thumbColsForSize(from);
  let size = from;
  while (true) {
    const next = size + dir;
    if (next < 0 || next > THUMB_MAX) return size;
    size = next;
    if (thumbColsForSize(size) !== startCols) return size;
  }
}

/**
 * @param {object} context
 * @param {() => string} context.currentUser
 * @param {() => boolean} context.embedded
 */
export function useAlbumThumbnailGrid({ currentUser, embedded }) {
  const viewLayout = ref(loadViewLayout(currentUser(), { mobile: embedded() }));
  const zoomChromeLock = ref(null);
  const photoGridRef = ref(null);

  const thumbnailSize = computed({
    get: () => viewLayout.value.thumbnailSize,
    set: (value) => {
      if (!zoomChromeLock.value) {
        zoomChromeLock.value = {
          compact: thumbsOnlyLook.value,
          thumbsOnly: thumbsOnlyLook.value,
          hideOverlays: hideCardOverlays.value,
        };
      }
      viewLayout.value.thumbnailSize = value;
      nextTick(() => {
        if (!photoGridRef.value?.isZoomBusy?.()) zoomChromeLock.value = null;
      });
    },
  });

  const forcedThumbsOnly = computed(() => thumbnailSize.value < THUMB_DETAILS_MIN);
  const thumbsOnlyLook = computed(
    () => viewLayout.value.compact || forcedThumbsOnly.value || !hasBelowInfo(viewLayout.value),
  );
  const hideCardOverlays = computed(() => forcedThumbsOnly.value);
  const gridCompact = computed(() => zoomChromeLock.value?.compact ?? thumbsOnlyLook.value);
  const gridHideOverlays = computed(() => zoomChromeLock.value?.hideOverlays ?? hideCardOverlays.value);
  const gridThumbsOnly = computed(() => zoomChromeLock.value?.thumbsOnly ?? thumbsOnlyLook.value);

  const thumbMaxIdx = THUMB_MAX;
  const thumbZoomClasses = Array.from(
    { length: THUMB_MAX + 1 },
    (_, size) => `grid ${THUMB_GRID_COLS[THUMB_MAX - size]} items-start gap-[var(--photo-gap)]`,
  );
  const thumbZoomGaps = computed(() =>
    Array.from({ length: THUMB_MAX + 1 }, (_, size) => {
      const thumbsOnly = viewLayout.value.compact || size < THUMB_DETAILS_MIN || !hasBelowInfo(viewLayout.value);
      return thumbsOnly ? 0 : (THUMB_GAP_PX[THUMB_MAX - size] ?? 6);
    }),
  );
  const thumbGapPx = computed(() => {
    if (thumbsOnlyLook.value) return 0;
    const idx = thumbMaxIdx - thumbnailSize.value;
    return THUMB_GAP_PX[idx] ?? 6;
  });
  const thumbGridClass = computed(() => {
    const idx = thumbMaxIdx - thumbnailSize.value;
    const cols = THUMB_GRID_COLS[idx] || THUMB_GRID_COLS[3];
    return `${cols} items-start gap-[var(--photo-gap)]`;
  });

  const thumbnailPopoverOpen = ref(false);
  const thumbnailBtnRef = ref(null);
  const thumbnailPopoverStyle = ref({});
  const viewSettingsOpen = ref(false);
  const thumbSizeToastOpen = ref(false);
  let thumbWheelAcc = 0;
  let thumbToastTimer = 0;

  function toggleThumbnailPopover() {
    thumbnailPopoverOpen.value = !thumbnailPopoverOpen.value;
    if (thumbnailPopoverOpen.value && thumbnailBtnRef.value) {
      const rect = thumbnailBtnRef.value.getBoundingClientRect();
      const pw = 288;
      let left = rect.left + rect.width / 2 - pw / 2;
      if (left + pw > window.innerWidth - 8) left = window.innerWidth - 8 - pw;
      if (left < 8) left = 8;
      thumbnailPopoverStyle.value = {
        position: "fixed",
        top: `${rect.bottom + 4}px`,
        left: `${left}px`,
      };
    }
  }

  function setViewCompact(compact) {
    if (!compact && thumbnailSize.value < THUMB_DETAILS_MIN) {
      viewLayout.value = {
        ...viewLayout.value,
        compact: false,
        thumbnailSize: THUMB_DETAILS_MIN,
      };
      return;
    }
    viewLayout.value = { ...viewLayout.value, compact };
  }

  function showThumbSizeToast() {
    thumbSizeToastOpen.value = true;
    window.clearTimeout(thumbToastTimer);
    thumbToastTimer = window.setTimeout(() => {
      thumbSizeToastOpen.value = false;
    }, 1400);
  }

  function stepThumbnailSize(event) {
    const raw = event.deltaY;
    const dy = event.deltaMode === 1 ? raw * 16 : event.deltaMode === 2 ? raw * 120 : raw;
    if (dy && thumbWheelAcc && Math.sign(dy) !== Math.sign(thumbWheelAcc)) {
      thumbWheelAcc = 0;
    }
    thumbWheelAcc += dy;
    if (Math.abs(thumbWheelAcc) < 36) return false;
    const dir = thumbWheelAcc > 0 ? -1 : 1;
    thumbWheelAcc = 0;
    const next = nextDistinctThumbSize(thumbnailSize.value, dir);
    if (next === thumbnailSize.value) return false;
    thumbnailSize.value = next;
    return true;
  }

  function onThumbSizeWheel(event) {
    stepThumbnailSize(event);
  }

  function openViewSettings() {
    thumbnailPopoverOpen.value = false;
    viewSettingsOpen.value = true;
  }

  function setViewLayout(layout) {
    viewLayout.value = layout;
  }

  watch(
    viewLayout,
    (layout) => {
      saveViewLayout(currentUser(), layout, { mobile: embedded() });
    },
    { deep: true },
  );

  watch(currentUser, (user) => {
    viewLayout.value = loadViewLayout(user, { mobile: embedded() });
  });

  return {
    photoGridRef,
    viewLayout,
    zoomChromeLock,
    thumbnailSize,
    forcedThumbsOnly,
    thumbsOnlyLook,
    hideCardOverlays,
    gridCompact,
    gridHideOverlays,
    gridThumbsOnly,
    thumbMaxIdx,
    thumbZoomClasses,
    thumbZoomGaps,
    thumbGapPx,
    thumbGridClass,
    thumbnailPopoverOpen,
    thumbnailBtnRef,
    thumbnailPopoverStyle,
    toggleThumbnailPopover,
    setViewCompact,
    thumbSizeToastOpen,
    showThumbSizeToast,
    stepThumbnailSize,
    onThumbSizeWheel,
    viewSettingsOpen,
    openViewSettings,
    setViewLayout,
  };
}
