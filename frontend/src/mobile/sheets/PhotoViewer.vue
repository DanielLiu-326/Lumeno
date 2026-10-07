<script setup>
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { t } from "../../i18n/index.js";
import { COLOR_LABELS } from "../../utils/photoCardLayout.js";
import { downloadFileByUrl } from "../../utils/downloadFile.js";
import { formatDurationMs, isVideoPhoto, playbackUrl } from "../../utils/mediaKind.js";
import {
  IconArrowLeft, IconCamera, IconCheck, IconChevronLeft, IconChevronRight,
  IconChevronUp, IconDots, IconDownload, IconImage, IconPencil, IconSparkles,
  IconStar, IconTrash, IconViewOriginal,
} from "../../utils/icons";
import ActionSheet from "./ActionSheet.vue";
import AppDialog from "../../components/AppDialog.vue";
import AppSlider from "../../components/AppSlider.vue";
import TagChips from "../../components/TagChips.vue";
import { useOverlayHistory } from "../useOverlayHistory.js";
import { usePhotoPinch } from "../usePhotoPinch.js";

const props = defineProps({
  photo: { type: Object, default: null },
  isDark: { type: Boolean, default: false },
  formatSize: { type: Function, required: true },
  canDelete: { type: Boolean, default: false },
  canWrite: { type: Boolean, default: true },
  albums: { type: Array, default: () => [] },
  sharedAlbums: { type: Array, default: () => [] },
  currentAlbum: { type: String, default: "" },
  hasPrev: { type: Boolean, default: false },
  hasNext: { type: Boolean, default: false },
  prevPhoto: { type: Object, default: null },
  nextPhoto: { type: Object, default: null },
  indexLabel: { type: String, default: "" },
  availableTags: { type: Array, default: () => [] },
});

const emit = defineEmits(["close", "prev", "next", "save", "delete", "set-cover", "copy-to-album", "find-similar"]);

const show = computed(() => !!props.photo);
useOverlayHistory(show, () => emit("close"));

const VIEW_PREFS_KEY = "album-mobile-view-preferences";

function loadViewPrefs() {
  try {
    return JSON.parse(localStorage.getItem(VIEW_PREFS_KEY)) || {};
  } catch {
    return {};
  }
}

function saveViewPrefs() {
  try {
    localStorage.setItem(VIEW_PREFS_KEY, JSON.stringify({
      original: prefOriginal.value,
      darktable: prefDarktable.value,
    }));
  } catch {
    // Ignore quota / private-mode failures.
  }
}

const savedViewPrefs = loadViewPrefs();
const editing = ref(false);
const prefOriginal = ref(!!savedViewPrefs.original || !!savedViewPrefs.fullscreen);
const prefDarktable = ref(!!savedViewPrefs.darktable);
const hasXmp = ref(false);
const viewOriginal = computed(() => prefOriginal.value);
const infoOpen = ref(false);
const infoDragging = ref(false);
const infoMode = ref("");
const infoRef = ref(null);
const dismissY = ref(0);
const dismissMode = ref("");
let dismissTimer = 0;
let infoTimer = 0;
let infoDragY = 0;
let infoOpenH = 0;
let infoLastY = 0;
let infoLastT = 0;
let infoVy = 0;
const darktablePreview = computed(() => prefDarktable.value && hasXmp.value);
const imageLoading = ref(false);
const imageEpoch = ref(0);
const imageFallback = ref(false);
const currentFullReady = ref(false);
const readySrcs = new Set();
const readyGen = ref(0);
const EMPTY_PREV = { path: "__empty_prev__" };
const EMPTY_NEXT = { path: "__empty_next__" };
const slideSnapping = ref(false);
const slideCommitting = ref(false);
const stageRef = ref(null);
const fitRef = ref(null);
const trackRef = ref(null);
const chromeVisible = ref(true);
const editRating = ref(0);
const editColorLabels = ref([]);
const editTags = ref([]);
const moreOpen = ref(false);
const viewOpen = ref(false);
const albumOpen = ref(false);
const exportOpen = ref(false);
const exporting = ref(false);
const exportForm = ref({ format: "jpeg", quality: 95, maxWidth: 0, applyXmp: false });
// Same format at full size with grading off: the backend sends the original
// file directly, no darktable pass.
const exportDirectCopy = computed(() => {
  const photo = props.photo;
  if (!photo || exportForm.value.applyXmp || exportForm.value.maxWidth) return false;
  const srcExt = (photo.name.split(".").pop() || "").toLowerCase();
  switch (exportForm.value.format) {
    case "jpeg": return srcExt === "jpg" || srcExt === "jpeg";
    case "png": return srcExt === "png";
    case "tiff": return srcExt === "tif" || srcExt === "tiff";
    case "webp": return srcExt === "webp";
    default: return false;
  }
});
let chromeTimer = null;
let snapTimer = 0;
let fromSlideCommit = false;
let keepSlideOffset = false;
let pageShifted = false;
let infoStartY = 0;
const videoRef = ref(null);
const videoPlaying = ref(false);
const videoUnplayable = ref(false);

function pauseViewerVideo() {
  videoPlaying.value = false;
  videoRef.value?.pause?.();
}

const {
  zoomed: photoZoomed,
  active: photoPinching,
  sliding: photoSliding,
  slideX,
  imgStyle: photoImgStyle,
  reset: resetPhotoZoom,
  resetZoom: resetPhotoScale,
  onTouchStart: onPhotoTouchStart,
  onTouchMove: onPhotoTouchMove,
  onTouchEnd: onPhotoTouchEnd,
} = usePhotoPinch({
  isLocked: () => editing.value || dismissMode.value === "out",
  allowPinch: () => !videoPlaying.value,
  canSlidePrev: () => !editing.value && (localStrip.value ? isSlidePhoto(localStrip.value[0]) : props.hasPrev),
  canSlideNext: () => !editing.value && (localStrip.value ? isSlidePhoto(localStrip.value[2]) : props.hasNext),
  canDismiss: () => !editing.value && dismissMode.value !== "out",
  onTap() {
    chromeVisible.value = !chromeVisible.value;
    if (chromeVisible.value) armChrome();
  },
  onSwipeUp() {
    if (dismissY.value > 8 || infoDragY > 8) return;
    openInfo();
  },
  onDismissMove(dy) {
    if (dismissMode.value === "out") return;
    if (infoOpen.value) {
      beginInfoDrag();
      paintInfoDrag(dy);
      return;
    }
    dismissMode.value = "drag";
    dismissY.value = dy;
    chromeVisible.value = false;
  },
  onSwipeDown(info) {
    if (infoOpen.value) {
      finishInfoDrag(info);
      return;
    }
    const dy = Number(info?.dy) || 0;
    const vy = Number(info?.vy) || 0;
    const h = window.innerHeight || 1;
    if (dy > 88 || vy > 0.65 || dy > h * 0.16) {
      finishDismiss();
      return;
    }
    snapDismissBack();
  },
  onSlideStart() {
    interruptSnap();
    pauseViewerVideo();
  },
  onSlideGrab() {
    interruptSnap();
  },
  onSlidePage(dir) {
    return shiftSlidePage(dir);
  },
  onSlideEnd(info) {
    finishSlide(info);
  },
});

watch(stageRef, (el, _, onCleanup) => {
  if (!el) return;
  el.addEventListener("touchmove", onPhotoTouchMove, { passive: false });
  onCleanup(() => el.removeEventListener("touchmove", onPhotoTouchMove));
});

const RAW_EXTS = new Set(["cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "raf", "pef", "srw"]);

const addableAlbums = computed(() => {
  const all = [
    ...props.albums.map((a) => ({ ...a, owner: null })),
    ...props.sharedAlbums.filter((a) => a.can_write),
  ];
  return all.filter((a) => a.dir_name !== props.currentAlbum);
});

const tags = computed(() => (props.photo?.tags || []).filter(Boolean));
const colorKeys = computed(() => {
  const value = props.photo?.color_label;
  const labels = Array.isArray(value) ? value : String(value || "").split(",");
  return labels.map((label) => String(label).trim()).filter(Boolean);
});
const cameraLabel = computed(() => [props.photo?.camera_make, props.photo?.camera_model].filter(Boolean).join(" "));
const dimensions = computed(() => {
  const w = props.photo?.width || 0;
  const h = props.photo?.height || 0;
  return w > 0 && h > 0 ? `${w} × ${h}` : "";
});
const dirty = computed(() => {
  if (!props.photo || !editing.value) return false;
  if (editRating.value !== (props.photo.rating || 0)) return true;
  const current = [...editColorLabels.value].sort().join(",");
  const original = [...colorKeys.value].sort().join(",");
  if (current !== original) return true;
  const nextTags = [...editTags.value].map((s) => s.trim()).filter(Boolean).sort().join(",");
  return nextTags !== [...tags.value].sort().join(",");
});

const photoOwner = computed(() => {
  if (props.photo?.owner) return props.photo.owner;
  const album = String(props.currentAlbum || "");
  const slash = album.indexOf("/");
  return slash > 0 ? album.slice(0, slash) : "";
});

const ownerQuery = computed(() => {
  const owner = photoOwner.value;
  return owner ? `owner=${encodeURIComponent(owner)}` : "";
});

function ownerAmpFor(photo) {
  const owner = photo?.owner || (photo?.path && photo.path === props.photo?.path ? photoOwner.value : "") || photoOwner.value;
  return owner ? `&owner=${encodeURIComponent(owner)}` : "";
}

function thumbSrc(photo, size = 360) {
  if (!photo?.path || String(photo.path).startsWith("__empty")) return "";
  return `/api/thumbnail/${encodeURIComponent(photo.path)}?size=${size}${ownerAmpFor(photo)}`;
}

function isSlidePhoto(item) {
  return !!item?.path && !String(item.path).startsWith("__empty");
}

const localStrip = ref(null);

const slideItems = computed(() => {
  if (localStrip.value) return localStrip.value;
  return [
    props.prevPhoto || EMPTY_PREV,
    props.photo,
    props.nextPhoto || EMPTY_NEXT,
  ];
});

function slideThumbSrc(item) {
  return isSlidePhoto(item) ? thumbSrc(item, 360) : "";
}

function slidePlaybackSrc(item) {
  if (!isSlidePhoto(item)) return "";
  return playbackUrl(item.path, item.owner || photoOwner.value);
}

function slideFullSrc(item) {
  if (!isSlidePhoto(item) || isVideoPhoto(item)) return "";
  // While the strip is moving/committing, keep neighbor URLs so Vue can reuse the same <img>.
  if (
    item.path === props.photo?.path
    && !slideCommitting.value
    && !photoSliding.value
    && !slideSnapping.value
  ) {
    return imageSrc.value;
  }
  return thumbSrc(item, 1600);
}

function slideZoomStyle(index) {
  if (index !== 1) return { transform: "translate3d(0,0,0) scale(1)" };
  return photoImgStyle.value;
}

const imageSrc = computed(() => {
  const photo = props.photo;
  if (!photo?.path) return "";
  const ownerAmp = ownerAmpFor(photo);
  if (darktablePreview.value && hasXmp.value) {
    return `/api/albums/darktable_preview/${encodeURIComponent(photo.path)}?size=0${ownerAmp}`;
  }
  if (imageFallback.value) {
    return `/api/thumbnail/${encodeURIComponent(photo.path)}?size=800${ownerAmp}`;
  }
  if (viewOriginal.value) {
    const ext = (photo.name || "").split(".").pop().toLowerCase();
    if (RAW_EXTS.has(ext)) {
      return `/api/thumbnail/${encodeURIComponent(photo.path)}?size=0${ownerAmp}`;
    }
    const q = ownerQuery.value ? `?${ownerQuery.value}` : "";
    return `/api/download/${encodeURIComponent(photo.path)}${q}`;
  }
  return `/api/thumbnail/${encodeURIComponent(photo.path)}?size=1600${ownerAmp}`;
});

const trackStyle = computed(() => ({
  transform: `translate3d(calc(-100% + ${slideX.value}px), 0, 0)`,
}));

const dismissProgress = computed(() => {
  const h = Math.max(320, window.innerHeight || 1);
  return Math.min(1, Math.max(0, dismissY.value) / (h * 0.9));
});

const viewerDismissStyle = computed(() => {
  if (!dismissMode.value && dismissY.value <= 0) return undefined;
  const p = dismissProgress.value;
  return {
    background: `rgb(11 13 18 / ${Math.max(0, 1 - p)})`,
  };
});

const stageDismissStyle = computed(() => {
  if (!dismissMode.value && dismissY.value <= 0) return undefined;
  const p = dismissProgress.value;
  const scale = Math.max(0.82, 1 - p * 0.12);
  return {
    transform: `translate3d(0, ${dismissY.value}px, 0) scale(${scale})`,
    background: "transparent",
  };
});

const dismissing = computed(() => dismissMode.value === "drag" || dismissMode.value === "out" || dismissY.value > 8);

const stageBusy = computed(() =>
  photoSliding.value
  || slideSnapping.value
  || slideCommitting.value
  || Math.abs(slideX.value) > 1
);

function markSrcReady(src) {
  if (!src || readySrcs.has(src)) return;
  readySrcs.add(src);
  readyGen.value += 1;
}

function srcIsReady(src) {
  return !!src && readyGen.value >= 0 && readySrcs.has(src);
}

const isCurrentVideo = computed(() => isVideoPhoto(props.photo));
const durationLabel = computed(() => formatDurationMs(props.photo?.duration_ms));

const moreActions = computed(() => {
  const video = isCurrentVideo.value;
  const items = [
    { id: "download", label: t("detail.download"), icon: IconDownload },
  ];
  if (!video) {
    items.push({ id: "export", label: t("detail.exportBtn"), icon: IconDownload });
  }
  if (props.canWrite && !video) {
    items.push({ id: "cover", label: t("detail.setAsCover"), icon: IconImage });
  }
  items.push(
    { id: "album", label: t("detail.addToAlbum"), icon: IconImage, disabled: addableAlbums.value.length === 0 },
  );
  if (!video) {
    items.push({ id: "similar", label: t("detail.findSimilar"), icon: IconSparkles });
  }
  if (props.canDelete) {
    items.push({ id: "delete", label: t("detail.delete"), icon: IconTrash, danger: true });
  }
  return items;
});

const albumActions = computed(() => addableAlbums.value.map((album) => ({
  id: album.dir_name,
  label: album.name || album.dir_name,
  subtitle: album.owner || "",
})));

function syncFields(photo) {
  editing.value = false;
  editRating.value = photo?.rating || 0;
  const value = photo?.color_label;
  const labels = Array.isArray(value) ? value : String(value || "").split(",");
  editColorLabels.value = labels.map((label) => String(label).trim()).filter(Boolean);
  editTags.value = (photo?.tags || []).filter(Boolean);
}

watch(
  () => props.photo?.path,
  () => {
    if (!fromSlideCommit) return;
    slideSnapping.value = false;
    if (!keepSlideOffset) slideX.value = 0;
  },
  { flush: "sync" },
);

watch(() => props.photo?.path, async (path) => {
  const fromSlide = fromSlideCommit;
  const keep = keepSlideOffset;
  fromSlideCommit = false;
  keepSlideOffset = false;
  pageShifted = false;
  localStrip.value = null;
  slideSnapping.value = false;
  if (!keep) slideX.value = 0;
  if (!fromSlide) resetPhotoZoom();
  if (!path) {
    hasXmp.value = false;
    moreOpen.value = false;
    viewOpen.value = false;
    albumOpen.value = false;
    exportOpen.value = false;
    infoOpen.value = false;
    infoDragging.value = false;
    infoMode.value = "";
    infoDragY = 0;
    infoOpenH = 0;
    clearInfoPaint();
    resetDismiss();
    return;
  }
  syncFields(props.photo);
  pauseViewerVideo();
  videoUnplayable.value = false;
  if (isVideoPhoto(props.photo)) viewOpen.value = false;
  imageFallback.value = false;
  currentFullReady.value = srcIsReady(imageSrc.value);
  imageLoading.value = !currentFullReady.value;
  hasXmp.value = false;
  if (infoOpen.value) {
    nextTick(() => paintInfoProgress(1));
  }
  await checkXmp(path, photoOwner.value, fromSlide);
});

watch(imageSrc, (src) => {
  currentFullReady.value = srcIsReady(src);
});

async function checkXmp(path, owner, fromSlide = false) {
  try {
    const q = owner ? `?owner=${encodeURIComponent(owner)}` : "";
    const res = await fetch(`/api/albums/has_xmp/${encodeURIComponent(path)}${q}`, { credentials: "same-origin" });
    if (!res.ok || props.photo?.path !== path) return;
    const data = await res.json();
    hasXmp.value = !!data.has_xmp;
    if (hasXmp.value && prefDarktable.value && !fromSlide) reloadImage();
  } catch {
    hasXmp.value = false;
  }
}

function tryClose() {
  if (editing.value && dirty.value && !window.confirm(t("msg.unsavedChangesClose"))) return;
  finishDismiss(true);
}

function clearDismissTimer() {
  window.clearTimeout(dismissTimer);
  dismissTimer = 0;
}

function resetDismiss() {
  clearDismissTimer();
  dismissY.value = 0;
  dismissMode.value = "";
}

function snapDismissBack() {
  clearDismissTimer();
  dismissMode.value = "snap";
  dismissY.value = 0;
  armChrome();
  dismissTimer = window.setTimeout(() => {
    dismissMode.value = "";
    dismissTimer = 0;
  }, 280);
}

function finishDismiss(confirmed = false) {
  if (!confirmed && editing.value && dirty.value && !window.confirm(t("msg.unsavedChangesClose"))) {
    snapDismissBack();
    return;
  }
  if (dismissMode.value === "out") {
    emit("close");
    return;
  }
  dismissMode.value = "out";
  chromeVisible.value = false;
  requestAnimationFrame(() => {
    dismissY.value = Math.round((window.innerHeight || 800) * 1.08);
  });
  clearDismissTimer();
  dismissTimer = window.setTimeout(() => {
    dismissTimer = 0;
    emit("close");
  }, 300);
}

function canLeavePhoto() {
  if (editing.value && dirty.value && !window.confirm(t("msg.unsavedChangesSwitch"))) return false;
  return true;
}

function stageWidth() {
  return stageRef.value?.clientWidth || window.innerWidth || 1;
}

let snapDone = null;
let slideRaf = 0;

function cancelSlideAnim() {
  if (slideRaf) {
    cancelAnimationFrame(slideRaf);
    slideRaf = 0;
  }
  window.clearTimeout(snapTimer);
  snapTimer = 0;
}

function interruptSnap() {
  if (!slideSnapping.value) return;
  cancelSlideAnim();
  slideSnapping.value = false;
  snapDone = null;
}

function finishSnap() {
  cancelSlideAnim();
  if (!slideSnapping.value && !snapDone) return;
  slideSnapping.value = false;
  const done = snapDone;
  snapDone = null;
  done?.();
}

function animateSlideTo(px, then, vx = 0) {
  cancelSlideAnim();
  snapDone = then || null;
  const from = slideX.value;
  const dist = px - from;
  if (Math.abs(dist) < 1.5) {
    slideSnapping.value = false;
    slideX.value = px;
    finishSnap();
    return;
  }
  const toward = Math.sign(dist);
  const approach = toward ? Math.max(0, vx * toward) : 0;
  let ms;
  if (approach > 0.22) {
    ms = Math.abs(dist) * 2.05 / approach;
  } else {
    ms = 80 + Math.abs(dist) * 0.38;
  }
  ms = Math.max(90, Math.min(340, ms));
  slideSnapping.value = true;
  const started = performance.now();
  const tick = (now) => {
    const t = Math.min(1, (now - started) / ms);
    const eased = 1 - (1 - t) ** 3;
    slideX.value = from + dist * eased;
    if (t < 1) {
      slideRaf = requestAnimationFrame(tick);
      return;
    }
    slideRaf = 0;
    slideX.value = px;
    finishSnap();
  };
  slideRaf = requestAnimationFrame(tick);
}

function applySlideCommit(dir) {
  if (slideCommitting.value) return;
  slideCommitting.value = true;
  slideSnapping.value = false;
  fromSlideCommit = true;
  keepSlideOffset = false;
  localStrip.value = null;
  if (dir === "next") emit("next");
  else emit("prev");
  slideX.value = 0;
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      slideCommitting.value = false;
    });
  });
}

function shiftSlidePage(dir) {
  if (pageShifted || editing.value) return false;
  if (dir === "next" && !props.hasNext) return false;
  if (dir === "prev" && !props.hasPrev) return false;
  if (!canLeavePhoto()) return false;
  const cur = slideItems.value;
  pageShifted = true;
  fromSlideCommit = true;
  keepSlideOffset = true;
  localStrip.value = dir === "next"
    ? [cur[1], cur[2], EMPTY_NEXT]
    : [EMPTY_PREV, cur[0], cur[1]];
  if (dir === "next") emit("next");
  else emit("prev");
  return true;
}

function finishSlide({ dx = 0, vx = 0, width = 0, cancelled = false } = {}) {
  if (cancelled) {
    animateSlideTo(0, null, vx);
    return;
  }
  const w = width || stageWidth();
  const goNext = props.hasNext && (dx < -w * 0.16 || (dx < -28 && vx < -0.32));
  const goPrev = props.hasPrev && (dx > w * 0.16 || (dx > 28 && vx > 0.32));
  if (goNext) commitSlide("next", w, vx);
  else if (goPrev) commitSlide("prev", w, vx);
  else animateSlideTo(0, null, vx);
}

function commitSlide(dir, width, vx = 0) {
  if (dir === "next" && !props.hasNext) return;
  if (dir === "prev" && !props.hasPrev) return;
  if (!canLeavePhoto()) {
    animateSlideTo(0, null, vx);
    return;
  }
  if (slideCommitting.value) return;
  resetPhotoScale();
  const w = width || stageWidth();
  animateSlideTo(dir === "next" ? -w : w, () => applySlideCommit(dir), vx);
}

function tryPrev() {
  commitSlide("prev");
}

function tryNext() {
  commitSlide("next");
}

function onSlideFullLoad(src) {
  markSrcReady(src);
  if (src && src === imageSrc.value) {
    imageLoading.value = false;
    currentFullReady.value = true;
  }
}

onBeforeUnmount(() => {
  cancelSlideAnim();
  clearDismissTimer();
  clearInfoTimer();
  snapDone = null;
});

function startEdit() {
  syncFields(props.photo);
  editing.value = true;
  openInfo();
}

function cancelEdit() {
  if (dirty.value && !window.confirm(t("msg.unsavedChangesClose"))) return;
  syncFields(props.photo);
}

function saveEdit() {
  const nextTags = [...editTags.value].map((s) => s.trim()).filter(Boolean);
  emit("save", {
    path: props.photo.path,
    album: props.photo.album,
    tags: nextTags,
    rating: editRating.value,
    colorLabel: [...editColorLabels.value],
  });
  props.photo.rating = editRating.value;
  props.photo.color_label = [...editColorLabels.value];
  props.photo.tags = nextTags;
  editing.value = false;
}

function setRating(star) {
  if (!editing.value) return;
  editRating.value = editRating.value === star ? 0 : star;
}

function toggleColor(key) {
  if (!editing.value) return;
  const idx = editColorLabels.value.indexOf(key);
  if (idx >= 0) editColorLabels.value.splice(idx, 1);
  else editColorLabels.value.push(key);
}

function reloadImage() {
  imageFallback.value = false;
  imageEpoch.value += 1;
  imageLoading.value = true;
  currentFullReady.value = false;
}

function setOriginal(on) {
  prefOriginal.value = !!on;
  saveViewPrefs();
  reloadImage();
}

function setDarktable(on) {
  prefDarktable.value = !!on;
  saveViewPrefs();
  reloadImage();
}

function infoViewer() {
  return infoRef.value?.parentElement || stageRef.value?.parentElement;
}

function infoPanelHeight() {
  return infoRef.value?.offsetHeight
    || Math.round((stageRef.value?.clientHeight || window.innerHeight || 800) * 0.46);
}

function photoNaturalSize() {
  const w = Number(props.photo?.width) || 0;
  const h = Number(props.photo?.height) || 0;
  if (w > 0 && h > 0) return { w, h };
  const img = stageRef.value?.querySelector(".mobile-photo-img.is-full");
  return {
    w: Number(img?.naturalWidth) || 0,
    h: Number(img?.naturalHeight) || 0,
  };
}

function containScale(boxW, boxH, imgW, imgH) {
  if (boxW <= 0 || boxH <= 0 || imgW <= 0 || imgH <= 0) return 1;
  return Math.min(boxW / imgW, boxH / imgH);
}

function paintInfoProgress(progress) {
  const p = Math.min(1, Math.max(0, progress));
  const viewer = infoViewer();
  if (viewer) viewer.style.setProperty("--info-open", String(p));

  const fit = fitRef.value;
  const stage = stageRef.value;
  if (!fit || !stage) return;
  if (p <= 0.001) {
    fit.style.transform = "translate3d(0, 0, 0) scale(1)";
    return;
  }
  const boxW = stage.clientWidth || 1;
  const boxH = stage.clientHeight || 1;
  const inset = (infoOpenH || infoPanelHeight()) * p;
  const remainH = Math.max(1, boxH - inset);
  const { w: imgW, h: imgH } = photoNaturalSize();
  const full = containScale(boxW, boxH, imgW, imgH);
  const remain = containScale(boxW, remainH, imgW, imgH);
  const scale = full > 0.0001 ? remain / full : 1;
  const shiftY = (remainH - boxH) / 2;
  fit.style.transform = `translate3d(0, ${shiftY}px, 0) scale(${scale})`;
}

function paintInfoDrag(dy) {
  const max = infoOpenH || infoPanelHeight();
  infoDragY = Math.min(max, Math.max(0, Number(dy) || 0));
  paintInfoProgress(1 - infoDragY / Math.max(1, max));
}

function clearInfoPaint() {
  infoViewer()?.style.removeProperty("--info-open");
  if (fitRef.value) fitRef.value.style.transform = "";
}

function openInfo() {
  clearInfoTimer();
  infoDragY = 0;
  infoDragging.value = false;
  infoMode.value = "snap-open";
  infoOpenH = 0;
  infoOpen.value = true;
  chromeVisible.value = true;
  if (chromeTimer) clearTimeout(chromeTimer);
  nextTick(() => {
    requestAnimationFrame(() => paintInfoProgress(1));
  });
  infoTimer = window.setTimeout(() => {
    infoMode.value = "";
    infoTimer = 0;
  }, 320);
}

function closeInfo() {
  if (editing.value) return;
  clearInfoTimer();
  infoDragY = 0;
  infoDragging.value = false;
  infoMode.value = "snap-close";
  infoOpenH = infoOpenH || infoPanelHeight();
  paintInfoProgress(0);
  infoOpen.value = false;
  infoTimer = window.setTimeout(() => {
    infoMode.value = "";
    infoOpenH = 0;
    infoTimer = 0;
    clearInfoPaint();
  }, 280);
}

function toggleInfo() {
  if (infoOpen.value) closeInfo();
  else openInfo();
}

function clearInfoTimer() {
  window.clearTimeout(infoTimer);
  infoTimer = 0;
}

function beginInfoDrag() {
  if (!infoDragging.value) {
    infoOpenH = infoPanelHeight();
  }
  infoDragging.value = true;
  infoMode.value = "";
}

function finishInfoDrag(info = {}) {
  if (editing.value) {
    infoDragging.value = false;
    infoDragY = 0;
    infoMode.value = "";
    clearInfoPaint();
    return;
  }
  const dy = Math.max(0, Number(info?.dy) || infoDragY || 0);
  const vy = Number(info?.vy) || 0;
  const openH = infoOpenH || infoPanelHeight();
  infoDragging.value = false;
  if (dy > 56 || vy > 0.55 || dy > openH * 0.28) {
    infoMode.value = "snap-close";
    clearInfoTimer();
    nextTick(() => {
      requestAnimationFrame(() => {
        paintInfoProgress(0);
      });
    });
    infoTimer = window.setTimeout(() => {
      infoTimer = 0;
      closeInfo();
    }, 280);
    return;
  }
  infoMode.value = "snap-open";
  infoDragY = 0;
  clearInfoTimer();
  nextTick(() => {
    requestAnimationFrame(() => {
      paintInfoProgress(1);
    });
  });
  infoTimer = window.setTimeout(() => {
    infoMode.value = "";
    infoTimer = 0;
    clearInfoPaint();
  }, 280);
}

function onInfoHandleStart(event) {
  if (editing.value) return;
  if (event.pointerType === "mouse" && event.button !== 0) return;
  infoStartY = event.clientY;
  infoLastY = event.clientY;
  infoLastT = performance.now();
  infoVy = 0;
  beginInfoDrag();
  paintInfoDrag(0);
  try {
    event.currentTarget.setPointerCapture?.(event.pointerId);
  } catch {
    // Capture can fail if the pointer already ended.
  }
}

function onInfoHandleMove(event) {
  if (!infoDragging.value) return;
  const now = performance.now();
  const dt = now - infoLastT;
  if (dt > 0) infoVy = (event.clientY - infoLastY) / dt;
  infoLastY = event.clientY;
  infoLastT = now;
  paintInfoDrag(event.clientY - infoStartY);
}

function onInfoHandleEnd() {
  if (!infoDragging.value && infoMode.value) return;
  if (!infoDragging.value) return;
  const dy = infoDragY;
  const vy = (performance.now() - infoLastT) > 80 ? 0 : infoVy;
  if (dy < 10) {
    infoDragging.value = false;
    infoDragY = 0;
    infoMode.value = "";
    clearInfoPaint();
    toggleInfo();
    return;
  }
  finishInfoDrag({ dy, vy });
}

function armChrome() {
  chromeVisible.value = true;
  if (chromeTimer) clearTimeout(chromeTimer);
  if (infoOpen.value || editing.value) return;
  chromeTimer = setTimeout(() => { chromeVisible.value = false; }, 2200);
}
function onVideoReady(event) {
  const el = event?.target;
  imageLoading.value = false;
  if (!(el instanceof HTMLVideoElement) || !el.isConnected) return;
  if (el.videoWidth > 0 && el.videoHeight > 0) return;
  el.pause();
  videoPlaying.value = false;
  videoUnplayable.value = true;
}
function onVideoPlay() {
  videoPlaying.value = true;
  imageLoading.value = false;
}
function onVideoPause() {
  videoPlaying.value = false;
}
function onVideoError(event) {
  const el = event?.target;
  if (el instanceof HTMLVideoElement && (!el.isConnected || (videoRef.value && el !== videoRef.value))) return;
  videoUnplayable.value = true;
  videoPlaying.value = false;
  imageLoading.value = false;
}
function onImageError() {
  if (!imageFallback.value) {
    imageFallback.value = true;
    imageEpoch.value += 1;
    imageLoading.value = true;
    return;
  }
  imageLoading.value = false;
}

function downloadOriginal() {
  if (!props.photo) return;
  const q = ownerQuery.value ? `?${ownerQuery.value}` : "";
  // Blob download: a plain <a href> navigation hands the file to Safari's
  // inline viewer on iOS even with an attachment header.
  downloadFileByUrl(
    `/api/download/${encodeURIComponent(props.photo.path)}${q}`,
    props.photo.name,
  ).catch((e) => {
    alert(e.message || "Download failed");
  });
}

async function doExport() {
  if (!props.photo || exporting.value) return;
  exporting.value = true;
  try {
    const body = {
      path: props.photo.path,
      format: exportForm.value.format,
      quality: exportForm.value.quality,
      max_width: exportForm.value.maxWidth || 0,
      apply_xmp: !!exportForm.value.applyXmp,
    };
    if (photoOwner.value) body.owner = photoOwner.value;
    const res = await fetch("/api/albums/export", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      credentials: "same-origin",
      body: JSON.stringify(body),
    });
    if (!res.ok) {
      const err = await res.json().catch(() => ({}));
      alert(err.message || "Export failed");
      return;
    }
    const blob = await res.blob();
    const disposition = res.headers.get("Content-Disposition") || "";
    const match = disposition.match(/filename="?([^"]+)"?/i);
    const filename = match?.[1] || "export.jpg";
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
    exportOpen.value = false;
  } catch (e) {
    alert(e.message || "Export failed");
  } finally {
    exporting.value = false;
  }
}

function onMore(action) {
  moreOpen.value = false;
  if (action.id === "download") downloadOriginal();
  else if (action.id === "export") {
    exportForm.value = { format: "jpeg", quality: 95, maxWidth: 0, applyXmp: hasXmp.value };
    exportOpen.value = true;
  } else if (action.id === "cover") emit("set-cover", props.photo);
  else if (action.id === "album") albumOpen.value = true;
  else if (action.id === "similar") {
    emit("close");
    emit("find-similar", props.photo);
  } else if (action.id === "delete") {
    if (!window.confirm(t("msg.confirmDeletePhoto", { name: props.photo.name }))) return;
    emit("delete", props.photo);
  }
}

function onPickAlbum(action) {
  albumOpen.value = false;
  emit("copy-to-album", { photoPath: props.photo.path, sourceAlbum: props.currentAlbum, targetAlbum: action.id });
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="photo"
      class="mobile-photo-viewer is-fullscreen"
      :class="{
        'is-dark': isDark,
        'is-slide-live': slideSnapping || slideCommitting,
        'is-info-open': infoOpen,
        'is-info-dragging': infoDragging,
        'is-info-snapping': infoMode === 'snap-open' || infoMode === 'snap-close',
        'is-dismiss-drag': dismissMode === 'drag',
        'is-dismiss-snap': dismissMode === 'snap',
        'is-dismiss-out': dismissMode === 'out',
      }"
      :style="viewerDismissStyle"
    >
      <header class="mobile-photo-chrome mobile-photo-top" :class="!chromeVisible || dismissing ? 'is-hidden' : ''">
        <button type="button" class="mobile-icon-btn" @click="tryClose">
          <IconArrowLeft class="h-5 w-5" />
        </button>
        <div class="min-w-0 flex-1 text-center">
          <p class="truncate text-sm font-semibold">{{ photo.name }}</p>
          <p v-if="indexLabel" class="text-[0.6875rem] opacity-60">{{ indexLabel }}</p>
        </div>
        <button v-if="!isCurrentVideo" type="button" class="mobile-icon-btn" :title="t('mobile.viewOptions')" @click="viewOpen = true">
          <IconViewOriginal class="h-5 w-5" />
        </button>
        <button type="button" class="mobile-icon-btn" @click="moreOpen = true">
          <IconDots class="h-5 w-5" />
        </button>
      </header>

      <div
        ref="stageRef"
        class="mobile-photo-stage"
        :class="{
          'is-zoomed': photoZoomed,
          'is-pinching': photoPinching,
          'is-sliding': stageBusy,
        }"
        :style="stageDismissStyle"
        @touchstart="onPhotoTouchStart"
        @touchend="onPhotoTouchEnd"
        @touchcancel="onPhotoTouchEnd"
      >
        <div ref="fitRef" class="mobile-photo-fit">
        <div ref="trackRef" class="mobile-photo-track" :style="trackStyle">
          <div
            v-for="(item, index) in slideItems"
            :key="item.path"
            class="mobile-photo-slide"
          >
            <div class="mobile-photo-zoom" :style="slideZoomStyle(index)">
              <img
                v-if="slideThumbSrc(item)"
                class="mobile-photo-img is-thumb"
                :src="slideThumbSrc(item)"
                alt=""
                draggable="false"
              />
              <img
                v-if="slideFullSrc(item)"
                class="mobile-photo-img is-full"
                :class="{ 'is-ready': srcIsReady(slideFullSrc(item)) }"
                :src="slideFullSrc(item)"
                alt=""
                draggable="false"
                @load="onSlideFullLoad(slideFullSrc(item))"
                @error="index === 1 ? onImageError : undefined"
              />
            </div>
          </div>
        </div>
        </div>
        <video
          v-if="isCurrentVideo && !videoUnplayable"
          ref="videoRef"
          :key="photo.path"
          class="mobile-photo-video"
          :class="{ 'is-concealed': stageBusy }"
          :src="slidePlaybackSrc(photo)"
          controls
          playsinline
          @play="onVideoPlay"
          @pause="onVideoPause"
          @ended="onVideoPause"
          @loadeddata="onVideoReady"
          @error="onVideoError"
        />
        <div v-if="isCurrentVideo && videoUnplayable" class="mobile-photo-video-fallback">
          <p>{{ t("detail.videoUnplayable") }}</p>
          <button type="button" @click.stop="downloadOriginal">{{ t("detail.download") }}</button>
        </div>
        <button
          type="button"
          class="mobile-photo-nav is-prev"
          :class="!chromeVisible ? 'is-hidden' : ''"
          :disabled="!hasPrev"
          @click.stop="tryPrev"
        >
          <IconChevronLeft class="h-6 w-6" />
        </button>
        <button
          type="button"
          class="mobile-photo-nav is-next"
          :class="!chromeVisible ? 'is-hidden' : ''"
          :disabled="!hasNext"
          @click.stop="tryNext"
        >
          <IconChevronRight class="h-6 w-6" />
        </button>
      </div>

      <button
        type="button"
        class="mobile-photo-peek"
        :class="!chromeVisible || infoOpen || dismissing ? 'is-hidden' : ''"
        :title="t('detail.showDetails')"
        @click="openInfo"
      >
        <IconChevronUp class="h-6 w-6" />
      </button>

      <section
        ref="infoRef"
        class="mobile-photo-info"
        :class="{
          'is-open': infoOpen,
          'is-dragging': infoDragging,
          'is-snapping': infoMode === 'snap-open' || infoMode === 'snap-close',
        }"
      >
        <button
          type="button"
          class="mobile-photo-info-handle"
          :title="t('detail.collapse')"
          @pointerdown="onInfoHandleStart"
          @pointermove="onInfoHandleMove"
          @pointerup="onInfoHandleEnd"
          @pointercancel="onInfoHandleEnd"
        >
          <IconChevronUp class="h-6 w-6 transition-transform" :class="infoOpen ? 'rotate-180' : ''" />
        </button>
        <div class="mobile-photo-info-scroll">
          <p class="truncate text-base font-semibold">{{ photo.name }}</p>

          <div class="mt-3">
            <p class="mobile-photo-kicker">{{ t("detail.rating") }}</p>
            <div class="mt-1 flex gap-1">
              <button
                v-for="star in 5"
                :key="star"
                type="button"
                class="p-1"
                :disabled="!editing"
                :class="star <= (editing ? editRating : (photo.rating || 0)) ? 'text-yellow-400' : 'opacity-30'"
                @click="setRating(star)"
              >
                <IconStar class="h-6 w-6" />
              </button>
            </div>
          </div>

          <div class="mt-3">
            <p class="mobile-photo-kicker">{{ t("detail.colorLabel") }}</p>
            <div class="mt-2 flex flex-wrap gap-2.5" :class="editing ? 'color-label-edit' : ''">
              <button
                v-for="cl in COLOR_LABELS"
                :key="cl.key"
                type="button"
                class="h-7 w-7 rounded-full"
                :class="[cl.color, (editing ? editColorLabels : colorKeys).includes(cl.key) ? `ring-2 ${cl.ring} ring-offset-2` : 'opacity-35']"
                :disabled="!editing"
                @click="toggleColor(cl.key)"
              ></button>
            </div>
            <p v-if="!editing && colorKeys.length === 0" class="mt-1 text-xs opacity-50">{{ t("mobile.noColor") }}</p>
          </div>

          <div class="mt-3">
            <p class="mobile-photo-kicker">{{ t("detail.tags") }}</p>
            <TagChips
              class="mt-1.5"
              v-model="editTags"
              :available="availableTags"
              :editable="editing"
              :is-dark="true"
            />
          </div>

          <div class="mt-4 grid grid-cols-2 gap-x-4 gap-y-1.5 text-xs opacity-80">
            <div v-if="photo.datetime || photo.date"><span class="opacity-60">{{ t("detail.date") }}</span> {{ photo.datetime || photo.date }}</div>
            <div v-if="cameraLabel"><span class="opacity-60">{{ t("detail.camera") }}</span> {{ cameraLabel }}</div>
            <div v-if="photo.lens"><span class="opacity-60">{{ t("detail.lens") }}</span> {{ photo.lens }}</div>
            <div v-if="photo.focal_length"><span class="opacity-60">{{ t("detail.focalLength") }}</span> {{ photo.focal_length }}</div>
            <div v-if="photo.aperture"><span class="opacity-60">{{ t("detail.aperture") }}</span> {{ photo.aperture }}</div>
            <div v-if="photo.shutter_speed"><span class="opacity-60">{{ t("detail.shutter") }}</span> {{ photo.shutter_speed }}</div>
            <div v-if="photo.iso"><span class="opacity-60">ISO</span> {{ photo.iso }}</div>
            <div><span class="opacity-60">{{ t("detail.size") }}</span> {{ formatSize(photo.size) }}</div>
            <div v-if="dimensions"><span class="opacity-60">{{ t("detail.dimensions") }}</span> {{ dimensions }}</div>
            <div v-if="durationLabel"><span class="opacity-60">{{ t("detail.duration") }}</span> {{ durationLabel }}</div>
          </div>
        </div>

        <div class="mobile-photo-footer">
          <template v-if="editing">
            <button type="button" class="mobile-photo-btn" @click="cancelEdit">{{ t("mobile.cancel") }}</button>
            <button type="button" class="mobile-photo-btn is-primary" @click="saveEdit">
              <IconCheck class="h-4 w-4" />{{ t("detail.save") }}
            </button>
          </template>
          <button v-else-if="canWrite" type="button" class="mobile-photo-btn is-primary" @click="startEdit">
            <IconPencil class="h-4 w-4" />{{ t("mobile.edit") }}
          </button>
        </div>
      </section>
    </div>
  </Teleport>

  <ActionSheet :show="moreOpen" :title="photo?.name" :actions="moreActions" @close="moreOpen = false" @select="onMore" />
  <ActionSheet :show="viewOpen" :title="t('mobile.viewOptions')" @close="viewOpen = false">
    <div class="mobile-sheet-toggle" @click="setOriginal(!viewOriginal)">
      <IconViewOriginal class="h-5 w-5 shrink-0 opacity-80" />
      <span class="min-w-0 flex-1 truncate text-left">{{ t("detail.viewOriginal") }}</span>
      <label class="app-switch" @click.stop>
        <input type="checkbox" :checked="viewOriginal" @change="setOriginal($event.target.checked)" />
        <span class="app-switch-track"></span>
      </label>
    </div>
    <div class="mobile-sheet-toggle" @click="setDarktable(!prefDarktable)">
      <IconCamera class="h-5 w-5 shrink-0 opacity-80" />
      <span class="min-w-0 flex-1 text-left">
        <span class="block truncate">{{ t("detail.darktablePreview") }}</span>
        <span v-if="!hasXmp" class="mt-0.5 block text-[11px] font-normal opacity-50">{{ t("detail.noXmp") }}</span>
      </span>
      <label class="app-switch" @click.stop>
        <input type="checkbox" :checked="prefDarktable" @change="setDarktable($event.target.checked)" />
        <span class="app-switch-track"></span>
      </label>
    </div>
  </ActionSheet>
  <ActionSheet :show="albumOpen" :title="t('detail.addToAlbum')" :actions="albumActions" @close="albumOpen = false" @select="onPickAlbum" />

  <AppDialog :show="exportOpen" :z-index="160" @close="exportOpen = false">
    <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-sm rounded-2xl border p-5" :class="isDark ? 'text-stone-100' : 'text-stone-900'">
      <h3 class="mb-4 text-base font-bold">{{ t("detail.exportTitle") }}</h3>
      <div class="mb-3">
        <label class="mb-1 block text-xs opacity-60">{{ t("detail.exportFormat") }}</label>
        <select v-model="exportForm.format" class="mobile-search-input w-full">
          <option value="jpeg">JPEG</option>
          <option value="png">PNG</option>
          <option value="tiff">TIFF</option>
          <option value="webp">WebP</option>
        </select>
      </div>
      <div v-if="exportForm.format === 'jpeg' || exportForm.format === 'webp'" class="mb-3">
        <label class="mb-1 block text-xs opacity-60">{{ t("detail.exportQuality") }}: {{ exportForm.quality }}</label>
        <AppSlider v-model="exportForm.quality" :min="1" :max="100" :is-dark="isDark" />
      </div>
      <div class="mb-4">
        <label class="mb-1 block text-xs opacity-60">{{ t("detail.exportMaxWidth") }}</label>
        <input v-model.number="exportForm.maxWidth" type="number" min="0" step="100" class="mobile-search-input w-full" :placeholder="t('detail.exportMaxWidthPlaceholder')" />
      </div>
      <div class="mb-4 flex items-center justify-between gap-3">
        <label class="text-xs" :class="hasXmp ? 'opacity-80' : 'opacity-50'">
          {{ t("detail.exportApplyXmp") }}
          <span v-if="!hasXmp" class="ml-1 font-normal opacity-70">{{ t("detail.noXmp") }}</span>
        </label>
        <label class="app-switch">
          <input type="checkbox" v-model="exportForm.applyXmp" :disabled="!hasXmp" />
          <span class="app-switch-track"></span>
        </label>
      </div>
      <p v-if="exportDirectCopy" class="mb-3 text-[11px] leading-snug opacity-50">
        {{ t("detail.exportDirectHint") }}
      </p>
      <div class="flex gap-2">
        <button type="button" class="mobile-photo-btn flex-1" @click="exportOpen = false">{{ t("share.close") }}</button>
        <button type="button" class="mobile-photo-btn is-primary flex-1" :disabled="exporting" @click="doExport">
          {{ exporting ? t("detail.exporting") : t("detail.exportBtn") }}
        </button>
      </div>
    </div>
  </AppDialog>
</template>
