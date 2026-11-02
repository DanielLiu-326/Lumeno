<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick, Transition } from "vue";
import {
  IconFolderAdd, IconRefresh, IconStar, IconStarOff, IconFilter, IconGrade, IconDatabase,
  IconCamera, IconClose, IconChevronLeft, IconChevronRight, IconExpand, IconCollapse,
  IconViewOriginal,
  IconArrowLeft, IconTrash, IconPencil, IconPlus, IconImage, IconCheck,
  IconImport, IconZoomIn, IconZoomOut, IconDownload, IconSparkles, IconSearch,   IconWand,
  IconGrid, IconList, IconGear, IconAlertCircle, IconSort,
} from "../utils/icons";
import { t } from "../i18n/index.js";
import { albumCoverUrl } from "../utils/albumCover.js";
import { DEFAULT_THUMBNAIL_SIZE, thumbnailUrl } from "../utils/thumbnailUrl.js";
import { encodePath } from "../utils/path.js";
import { httpFetch, httpJson, postJson } from "../utils/http";
import { downloadFileByUrl, saveBlobResponse } from "../utils/downloadFile.js";
import { gradedThumbEpoch, gradedThumbRev, markGradedThumb } from "../utils/gradedThumb.js";
import { formatDurationMs, isVideoPhoto, playbackUrl } from "../utils/mediaKind.js";
import { useAlbumThumbnailGrid } from "../composables/useAlbumThumbnailGrid.js";
import { loadVideoInfo, planPlayback, startVideoEncode, watchVideoEncode } from "../utils/videoEncode.js";
import ImportSourceModal from "./ImportSourceModal.vue";
import CloudFilePicker from "./CloudFilePicker.vue";
import FilterSidebar from "./FilterSidebar.vue";
import FilterSheet from "../mobile/sheets/FilterSheet.vue";
import SortSheet from "../mobile/sheets/SortSheet.vue";
import ActionSheet from "../mobile/sheets/ActionSheet.vue";
import AlbumAiToolsModal from "./AlbumAiToolsModal.vue";
import VirtualGrid from "./VirtualGrid.vue";
import LazyImage from "./LazyImage.vue";
import { VList } from "virtua/vue";
import PhotoCard from "./PhotoCard.vue";
import PhotoMetaPanel from "./PhotoMetaPanel.vue";
import GradeWorkspace from "./GradeWorkspace.vue";
import GradeResources from "./grade/GradeResources.vue";
import ViewSettingsDialog from "./ViewSettingsDialog.vue";
import AppDialog from "./AppDialog.vue";
import AppSlider from "./AppSlider.vue";
import TagChips from "./TagChips.vue";
import { COLOR_LABELS, THUMB_DEFAULT, THUMB_DETAILS_MIN } from "../utils/photoCardLayout.js";

const props = defineProps({
  isDark: { type: Boolean, default: false },
  busyAlbums: { type: Boolean, default: false },
  albums: { type: Array, required: true },
  sharedAlbums: { type: Array, default: () => [] },
  currentAlbum: { type: String, default: "" },
  currentUser: { type: String, default: "" },
  albumViewMode: { type: String, default: "photos" },
  albumPhotos: { type: Array, required: true },
  duplicateGroups: { type: Array, default: () => [] },
  duplicateTotal: { type: Number, default: 0 },
  similarResults: { type: Array, default: () => [] },
  similarQuery: { type: Object, default: null },
  similarLoading: { type: Boolean, default: false },
  availableTags: { type: Array, required: true },
  availableCameras: { type: Array, default: () => [] },
  availableLenses: { type: Array, default: () => [] },
  availableFormats: { type: Array, default: () => [] },
  dateHistogram: { type: Array, default: () => [] },
  filters: { type: Object, required: true },
  isolateFilters: { type: Boolean, default: false },
  emptyLabel: { type: String, default: "" },
  sort: { type: Array, default: () => [] },
  formatSize: { type: Function, required: true },
  formatTime: { type: Function, required: true },
  fetchList: { type: Function, required: true },
  loadAllPhotos: { type: Function, default: null },
  collections: { type: Array, default: () => [] },
  selectedCollectionId: { type: String, default: "all" },
  albumPeople: { type: Array, default: () => [] },
  albumPeopleLoading: { type: Boolean, default: false },
  albumFaceHintsLoading: { type: Boolean, default: false },
  albumFaceSuggestions: { type: Object, default: () => ({ clusters: [] }) },
  albumFacesEditable: { type: Boolean, default: true },
  albumFacesOwner: { type: String, default: "" },
  albumClusterSimilarity: { type: Number, default: 0.62 },
  albumClusterReclustering: { type: Boolean, default: false },
  onCreateAlbum: { type: Function, required: true },
  onUpdateAlbum: { type: Function, required: true },
  onShareAlbum: { type: Function, required: true },
  onUnshareAlbum: { type: Function, required: true },
  /** In-modal AI search; must not switch albumViewMode. */
  onSearchTextInModal: { type: Function, default: null },
  /** In-modal duplicates load; must not switch albumViewMode. */
  onLoadDuplicatesInModal: { type: Function, default: null },
  /** Fetch album-index metadata for paths (EXIF for AI-tool photo opens). */
  fetchPhotosByPaths: { type: Function, default: null },
  /** Phone shell: hide list + desktop toolbar, keep grid/detail/dialogs. */
  embedded: { type: Boolean, default: false },
  /** Pause virtualized grid work while the album overlay is off-screen. */
  gridPaused: { type: Boolean, default: false },
});

const emit = defineEmits([
  "refresh-albums",
  "select-album",
  "delete-album",
  "show-duplicates",
  "show-photos",
  "find-similar",
  "search-text",
  "apply-filters",
  "update-sort",
  "import-selected",
  "import-local",
  "import-cloud",
  "update-photo-meta",
  "copy-photo-to-album",
  "rebuild-index",
  "load-more-photos",
  "delete-photos",
  "select-collection",
  "refresh-album-people",
  "patch-album-person",
  "remove-album-center",
  "delete-album-person",
  "adopt-album-cluster",
  "recluster-album-faces",
  "merge-album-cluster",
  "detail-photo",
]);

const {
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
} = useAlbumThumbnailGrid({
  currentUser: () => props.currentUser,
  embedded: () => props.embedded,
});

const importSourceModalOpen = ref(false);
const cloudPickerOpen = ref(false);
const localFileInput = ref(null);

function openImportModal() {
  importSourceModalOpen.value = true;
}

function handleImportLocal() {
  importSourceModalOpen.value = false;
  localFileInput.value?.click();
}

function handleImportCloud() {
  importSourceModalOpen.value = false;
  cloudPickerOpen.value = true;
}

function handleCloudPickerConfirm(paths) {
  cloudPickerOpen.value = false;
  if (paths && paths.length > 0) {
    emit('import-cloud', paths);
  }
}

function handleLocalFileChange(event) {
  const files = Array.from(event.target.files || []);
  if (files.length > 0) {
    emit('import-local', files);
  }
  event.target.value = "";
}


const isMobile = ref(!!props.embedded);
const filterSortPanelOpen = ref(false);
const filterPanelOpen = ref(false);
const sortPanelOpen = ref(false);
const aiToolsPanelOpen = ref(false);
const aiToolsPickerOpen = ref(false);
const aiToolsInitialSection = ref("faces");
const mobileSearchOpen = ref(false);
const mobileSearchInputRef = ref(null);
const gradeView = ref(false);
const resourcesOpen = ref(false);
const gradePhoto = ref(null);
let gradeEnrichToken = 0;
const photoPaneRef = ref(null);
const pinchLive = ref(false);
let pinchSession = null;

function rememberZoomFocal(event) {
  const rect = photoGridRef.value?.getHostRect?.();
  if (!rect) return;
  photoGridRef.value?.setZoomFocal?.(event.clientX - rect.left, event.clientY - rect.top);
}

function onGridZooming(active) {
  if (active) return;
  zoomChromeLock.value = null;
  pinchSession = null;
  pinchLive.value = false;
  unbindPinchWindowGuards();
}

function preventAlbumDragLock(event) {
  if (event.button !== 0) return;
  const target = event.target;
  if (!(target instanceof Element)) return;
  if (target.closest("input, textarea, select, button, a, label, [contenteditable='true']")) return;
  event.preventDefault();
}

function onPhotoPaneWheel(event) {
  if (!(event.ctrlKey || event.metaKey)) return;
  if (inSpecialView.value || detailPhoto.value) return;
  event.preventDefault();
  rememberZoomFocal(event);
  stepThumbnailSize(event);
  showThumbSizeToast();
}

function photoPaneTouchDistance(touches) {
  return Math.hypot(
    touches[0].clientX - touches[1].clientX,
    touches[0].clientY - touches[1].clientY,
  );
}

function photoPaneTouchCenter(touches) {
  const rect = photoGridRef.value?.getHostRect?.()
    || photoPaneRef.value?.getBoundingClientRect();
  if (!rect) return { x: 0, y: 0 };
  return {
    x: (touches[0].clientX + touches[1].clientX) / 2 - rect.left,
    y: (touches[0].clientY + touches[1].clientY) / 2 - rect.top,
  };
}

function onPhotoPaneTouchStart(event) {
  if (inSpecialView.value || detailPhoto.value) return;
  if (event.touches.length < 2) return;
  const grid = photoGridRef.value;
  if (!grid) return;
  if (!pinchSession && grid.isZoomBusy?.()) grid.resetInteraction?.();
  const center = photoPaneTouchCenter(event.touches);
  const started = grid.beginPinch?.({
    focalX: center.x,
    focalY: center.y,
  });
  if (!started) return;
  event.preventDefault();
  pinchLive.value = true;
  pinchSession = {
    startDist: photoPaneTouchDistance(event.touches),
    startSize: thumbnailSize.value,
    scale: 1,
  };
  bindPinchWindowGuards();
}

function onPhotoPaneTouchMove(event) {
  if (!pinchSession || event.touches.length < 2) return;
  event.preventDefault();
  const scale = photoPaneTouchDistance(event.touches) / Math.max(pinchSession.startDist, 1);
  const center = photoPaneTouchCenter(event.touches);
  pinchSession.scale = scale;
  photoGridRef.value?.updatePinch?.({
    scale,
    focalX: center.x,
    focalY: center.y,
  });
}

function onPhotoPaneTouchEnd(event) {
  if (!pinchSession) return;
  if (event.touches.length >= 2) return;
  pinchSession = null;
  pinchLive.value = false;
  unbindPinchWindowGuards();
  if (event.type === "touchcancel") {
    photoGridRef.value?.cancelPinch?.();
    return;
  }
  photoGridRef.value?.commitPinchSnap?.({
    commit: (next) => {
      thumbnailSize.value = next;
      showThumbSizeToast();
    },
  });
}

let pinchWindowBound = false;

function bindPinchWindowGuards() {
  if (pinchWindowBound) return;
  pinchWindowBound = true;
  const opts = { capture: true };
  window.addEventListener("touchend", onPhotoPaneTouchEnd, opts);
  window.addEventListener("touchcancel", onPhotoPaneTouchEnd, opts);
}

function unbindPinchWindowGuards() {
  if (!pinchWindowBound) return;
  pinchWindowBound = false;
  const opts = { capture: true };
  window.removeEventListener("touchend", onPhotoPaneTouchEnd, opts);
  window.removeEventListener("touchcancel", onPhotoPaneTouchEnd, opts);
}

function bindPhotoPaneGestures(el) {
  if (!el) return;
  const opts = { passive: false, capture: true };
  el.addEventListener("touchstart", onPhotoPaneTouchStart, opts);
  el.addEventListener("touchmove", onPhotoPaneTouchMove, opts);
  el.addEventListener("touchend", onPhotoPaneTouchEnd, opts);
  el.addEventListener("touchcancel", onPhotoPaneTouchEnd, opts);
}

function unbindPhotoPaneGestures(el) {
  if (!el) return;
  const opts = { passive: false, capture: true };
  el.removeEventListener("touchstart", onPhotoPaneTouchStart, opts);
  el.removeEventListener("touchmove", onPhotoPaneTouchMove, opts);
  el.removeEventListener("touchend", onPhotoPaneTouchEnd, opts);
  el.removeEventListener("touchcancel", onPhotoPaneTouchEnd, opts);
  unbindPinchWindowGuards();
}

function onThumbToastWheel(event) {
  event.preventDefault();
  stepThumbnailSize(event);
  showThumbSizeToast();
}

const detailPhoto = ref(null);
const detailPanelRef = ref(null);
watch(detailPhoto, (p) => {
  emit("detail-photo", p);
  if (!p || props.embedded) return;
  desktopSearchInputRef.value?.blur();
  nextTick(() => {
    detailPanelRef.value?.focus({ preventScroll: true });
  });
});
const detailClosing = ref(false);
/** When set, detail prev/next navigates this list instead of albumPhotos (AI tools context). */
const detailNavPhotos = ref(null);
const detailExpanded = ref(false);
const detailFullscreen = ref(false);
const detailFullscreenControlsVisible = ref(false);
const detailDimensions = ref(null);
const detailImageFallback = ref(false);
const detailImageEpoch = ref(0);
const detailVideoRef = ref(null);
const videoUnplayable = ref(false);
const detailVideoLive = ref(false);
const detailVideoPlan = ref(null);
const detailVideoEncoding = ref(false);
let detailVideoLiveTimer = 0;
let detailVideoPlanToken = 0;
let stopDetailEncodeWatch = () => {};

async function refreshDetailVideoPlan(photo) {
  const token = ++detailVideoPlanToken;
  stopDetailEncodeWatch();
  detailVideoPlan.value = null;
  detailVideoEncoding.value = false;
  if (!photo || !isVideoPhoto(photo)) return;
  const owner = photo.owner || currentAlbumOwner.value;
  try {
    const info = await loadVideoInfo(photo.path, owner, { refresh: true });
    if (token !== detailVideoPlanToken || detailPhoto.value?.path !== photo.path) return;
    detailVideoPlan.value = planPlayback(info);
  } catch {
    if (token !== detailVideoPlanToken || detailPhoto.value?.path !== photo.path) return;
    detailVideoPlan.value = planPlayback(null);
  }
}

async function confirmDetailVideoEncode() {
  const photo = detailPhoto.value;
  const plan = detailVideoPlan.value;
  if (!photo || plan?.mode !== "ask" || detailVideoEncoding.value) return;
  const owner = photo.owner || currentAlbumOwner.value;
  const token = detailVideoPlanToken;
  detailVideoEncoding.value = true;
  try {
    const started = await startVideoEncode(photo.path, owner, plan.codec);
    if (started?.ready) {
      const info = await loadVideoInfo(photo.path, owner, { refresh: true });
      if (token !== detailVideoPlanToken) return;
      detailVideoEncoding.value = false;
      detailVideoPlan.value = planPlayback(info);
      return;
    }
    stopDetailEncodeWatch = watchVideoEncode(photo.path, owner, plan.codec, started?.task_id, () => (
      token === detailVideoPlanToken && detailPhoto.value?.path === photo.path
    ), (next) => {
      detailVideoEncoding.value = false;
      videoUnplayable.value = false;
      detailVideoPlan.value = next;
    }, () => {
      if (token === detailVideoPlanToken) detailVideoEncoding.value = false;
    });
  } catch {
    if (token === detailVideoPlanToken) detailVideoEncoding.value = false;
  }
}

watch(() => detailPhoto.value?.path, (path) => {
  detailVideoLive.value = false;
  window.clearTimeout(detailVideoLiveTimer);
  refreshDetailVideoPlan(detailPhoto.value);
  if (!path || !isVideoPhoto(detailPhoto.value)) return;
  // Mount after the dialog fade. A video created while an ancestor opacity is
  // animating stays on the poster and only outputs audio.
  detailVideoLiveTimer = window.setTimeout(() => {
    if (detailPhoto.value?.path === path) detailVideoLive.value = true;
  }, 320);
});

onBeforeUnmount(() => {
  window.clearTimeout(detailVideoLiveTimer);
  stopDetailEncodeWatch();
});
const DETAIL_VIEW_PREFERENCES_KEY = "album-detail-view-preferences";

function loadDetailViewPreferences() {
  try {
    return JSON.parse(localStorage.getItem(DETAIL_VIEW_PREFERENCES_KEY)) || {};
  } catch {
    return {};
  }
}

const savedDetailViewPreferences = loadDetailViewPreferences();
const detailOriginalPreference = ref(savedDetailViewPreferences.original ?? true);
const detailDarktablePreference = ref(savedDetailViewPreferences.darktable ?? true);
const detailFullscreenPreference = ref(savedDetailViewPreferences.fullscreen ?? false);

function saveDetailViewPreferences() {
  localStorage.setItem(DETAIL_VIEW_PREFERENCES_KEY, JSON.stringify({
    original: detailOriginalPreference.value,
    darktable: detailDarktablePreference.value,
    fullscreen: detailFullscreenPreference.value,
  }));
}

const selectMode = ref(false);
const selectedPhotoPaths = ref(new Set());
const editRating = ref(0);
const editColorLabels = ref([]);
const editTags = ref([]);
const imageLoading = ref(false);

// Darktable preview / export
const darktablePreview = ref(false);
const hasXmpSidecar = ref(false);
const showExportDialog = ref(false);
const exporting = ref(false);
const exportForm = ref({ format: "jpeg", quality: 95, maxWidth: 0, applyXmp: false, applyGrade: false });
// The photo the export dialog targets (grid menu photo wins over the detail photo),
// plus its XMP state and whether the current settings allow a direct original copy.
const exportSelection = ref(false);
const exportSelectionPending = ref(false);
const exportPhoto = computed(() => menuPhoto.value || detailPhoto.value);
const exportHasXmp = computed(() => exportSelection.value || (menuPhoto.value ? menuHasXmp.value : hasXmpSidecar.value));
const exportDirectCopy = computed(() => {
  if (exportSelection.value) return false;
  const photo = exportPhoto.value;
  if (!photo || exportForm.value.applyXmp || exportForm.value.applyGrade) return false;
  if (exportForm.value.maxWidth) return false;
  const srcExt = (photo.name.split(".").pop() || "").toLowerCase();
  switch (exportForm.value.format) {
    case "jpeg": return srcExt === "jpg" || srcExt === "jpeg";
    case "png": return srcExt === "png";
    case "tiff": return srcExt === "tif" || srcExt === "tiff";
    case "webp": return srcExt === "webp";
    default: return false;
  }
});

// Add-to-album modal state
const showAddToAlbumModal = ref(false);
const addToAlbumPhoto = ref(null);
const addToAlbumLoading = ref(false);

// Album create/edit dialog state
const showAlbumDialog = ref(false);
const albumDialogMode = ref("create"); // "create" | "edit"
const albumDialogForm = ref({ dirName: "", name: "", description: "", cover: "" });

// Share dialog state
const showShareDialog = ref(false);
const shareDialogAlbum = ref(null);
const shareForm = ref({ username: "", canWrite: false, canDelete: false });
const shareMutationPending = ref(false);

function updateMobile() {
  if (props.embedded) {
    isMobile.value = true;
    return;
  }
  isMobile.value = window.innerWidth < 768
    || window.screen.width < 768
    || window.navigator.maxTouchPoints > 0
    || window.matchMedia("(pointer: coarse)").matches;
}

onMounted(() => {
  updateMobile();
  filterSortPanelOpen.value = props.embedded ? false : !isMobile.value;
  window.addEventListener("resize", updateMobile);
  window.addEventListener("keydown", onDetailKeydown);
});
onBeforeUnmount(() => {
  window.removeEventListener("resize", updateMobile);
  window.removeEventListener("keydown", onDetailKeydown);
  window.clearTimeout(thumbToastTimer);
  unbindPhotoPaneGestures(photoPaneRef.value);
  hideFullscreenControls();
});

function colorLabels(value) {
  const labels = Array.isArray(value) ? value : String(value || "").split(",");
  return labels.map((label) => String(label).trim()).filter(Boolean);
}

// ── Album list view helpers ──
const inAlbumView = computed(() => !!props.currentAlbum);
const currentAlbumObj = computed(() =>
  props.albums.find((a) => a.dir_name === props.currentAlbum)
  || props.sharedAlbums.find((a) => a.dir_name === props.currentAlbum)
);

const albumHeroId = ref("");
const canAlbumVt = typeof document !== "undefined"
  && typeof document.startViewTransition === "function";

function prefersReducedMotion() {
  return typeof window !== "undefined"
    && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function albumHeroStyle(dirName) {
  if (!dirName || albumHeroId.value !== dirName) return undefined;
  return { viewTransitionName: "album-hero" };
}

const ALBUM_VT_MS = 340;
const ALBUM_FETCH_LEAD_MS = 80;
const ALBUM_PHOTO_LOAD_MIN_MS = 600;
const albumPhotoLoadHold = ref(false);
let albumPhotoLoadToken = 0;
let albumPhotoLoadTimer = 0;
let stopAlbumPhotoLoadWatch = null;

function beginAlbumPhotoLoadHold() {
  if (albumPhotoLoadTimer) {
    window.clearTimeout(albumPhotoLoadTimer);
    albumPhotoLoadTimer = 0;
  }
  stopAlbumPhotoLoadWatch?.();
  stopAlbumPhotoLoadWatch = null;
  const token = ++albumPhotoLoadToken;
  const startedAt = performance.now();
  albumPhotoLoadHold.value = true;
  stopAlbumPhotoLoadWatch = watch(
    () => props.busyAlbums,
    (busy) => {
      if (busy || token !== albumPhotoLoadToken) return;
      stopAlbumPhotoLoadWatch?.();
      stopAlbumPhotoLoadWatch = null;
      const remain = Math.max(0, ALBUM_PHOTO_LOAD_MIN_MS - (performance.now() - startedAt));
      albumPhotoLoadTimer = window.setTimeout(() => {
        albumPhotoLoadTimer = 0;
        if (token !== albumPhotoLoadToken) return;
        albumPhotoLoadHold.value = false;
      }, remain);
    },
    { immediate: true },
  );
}

function endAlbumPhotoLoadHold() {
  albumPhotoLoadToken += 1;
  if (albumPhotoLoadTimer) {
    window.clearTimeout(albumPhotoLoadTimer);
    albumPhotoLoadTimer = 0;
  }
  stopAlbumPhotoLoadWatch?.();
  stopAlbumPhotoLoadWatch = null;
  albumPhotoLoadHold.value = false;
}

watch(
  () => props.currentAlbum,
  (album, previous) => {
    if (!album) {
      endAlbumPhotoLoadHold();
      return;
    }
    if (album === previous) return;
    if (previous === undefined && !props.busyAlbums && props.albumPhotos.length > 0) return;
    beginAlbumPhotoLoadHold();
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  endAlbumPhotoLoadHold();
});

function albumEnterDeferMs() {
  if (!canAlbumVt || prefersReducedMotion()) return 0;
  return Math.max(0, ALBUM_VT_MS - ALBUM_FETCH_LEAD_MS);
}

async function runAlbumViewTransition(mutate) {
  if (!canAlbumVt || prefersReducedMotion()) {
    await mutate();
    return;
  }
  const transition = document.startViewTransition(async () => {
    await mutate();
    await nextTick();
  });
  try {
    await transition.finished;
  } catch {
    /* aborted */
  }
}

function openCreateDialog() {
  albumDialogMode.value = "create";
  albumDialogForm.value = { dirName: "", name: "", description: "", cover: "" };
  showAlbumDialog.value = true;
}

function openEditDialog(album) {
  albumDialogMode.value = "edit";
  albumDialogForm.value = {
    dirName: album.dir_name,
    name: album.name || album.dir_name,
    description: album.description || "",
    cover: album.cover || "",
  };
  showAlbumDialog.value = true;
}

async function submitAlbumDialog() {
  const form = albumDialogForm.value;
  if (!form.name.trim()) return;
  let success = true;
  if (albumDialogMode.value === "create") {
    success = await props.onCreateAlbum({
      name: form.name.trim(),
      description: form.description,
    });
  } else {
    success = await props.onUpdateAlbum({
      dirName: form.dirName,
      name: form.name.trim(),
      description: form.description,
      cover: form.cover,
    });
  }
  if (success !== false) {
    showAlbumDialog.value = false;
  }
}

function confirmDeleteAlbum(album) {
  if (!window.confirm(t('msg.confirmDeleteAlbum', { name: album.name || album.dir_name }))) return;
  emit("delete-album", album.dir_name);
}

function openShareDialog(album) {
  shareDialogAlbum.value = { ...album, shares: album.shares ? album.shares.map(s => ({ ...s })) : [] };
  shareForm.value = { username: "", canWrite: false, canDelete: false };
  showShareDialog.value = true;
}

function syncShareDialogAlbum() {
  if (!showShareDialog.value || !shareDialogAlbum.value) {
    return;
  }
  const nextAlbum = props.albums.find((album) => album.dir_name === shareDialogAlbum.value.dir_name);
  if (!nextAlbum) {
    return;
  }
  shareDialogAlbum.value = {
    ...nextAlbum,
    shares: nextAlbum.shares ? nextAlbum.shares.map((share) => ({ ...share })) : [],
  };
}

watch(() => props.albums, syncShareDialogAlbum, { deep: true });
watch(() => props.albumViewMode, (mode) => {
  if (mode === 'photos') return;
  selectMode.value = false;
  selectedPhotoPaths.value = new Set();
  // Keep filterSortPanelOpen so returning from search restores the panel.
  mobileSearchOpen.value = false;
  thumbnailPopoverOpen.value = false;
});

async function submitShare() {
  const u = shareForm.value.username.trim();
  if (!u || !shareDialogAlbum.value || shareMutationPending.value) return;
  shareMutationPending.value = true;
  try {
    let success = true;
    const payload = {
      dirName: shareDialogAlbum.value.dir_name,
      username: u,
      canWrite: shareForm.value.canWrite,
      canDelete: shareForm.value.canDelete,
    };
    success = await props.onShareAlbum(payload);
    if (success !== false) {
      shareForm.value = { username: "", canWrite: false, canDelete: false };
      syncShareDialogAlbum();
    }
  } finally {
    shareMutationPending.value = false;
  }
}

function setShareFormWrite(value) {
  shareForm.value.canWrite = value;
  if (!value) shareForm.value.canDelete = false;
}

function setShareFormDelete(value) {
  shareForm.value.canDelete = value;
  if (value) shareForm.value.canWrite = true;
}

async function updateSharePermission(share, field, value) {
  if (!shareDialogAlbum.value || shareMutationPending.value) return;
  shareMutationPending.value = true;
  try {
    let canWrite = share.can_write;
    let canDelete = share.can_delete;
    if (field === "can_write") {
      canWrite = value;
      if (!value) canDelete = false;
    } else {
      canDelete = value;
      if (value) canWrite = true;
    }
    const payload = {
      dirName: shareDialogAlbum.value.dir_name,
      username: share.username,
      canWrite,
      canDelete,
    };
    await props.onShareAlbum(payload);
    syncShareDialogAlbum();
  } finally {
    shareMutationPending.value = false;
  }
}

async function removeShare(album, username) {
  if (shareMutationPending.value) return;
  shareMutationPending.value = true;
  try {
    await props.onUnshareAlbum({ dirName: album.dir_name, username });
    syncShareDialogAlbum();
  } finally {
    shareMutationPending.value = false;
  }
}

// Compute whether current album is a shared (not owned) album
const isSharedAlbum = computed(() => {
  if (!props.currentAlbum) return false;
  return props.currentAlbum.includes("/");
});

const currentAlbumOwner = computed(() => {
  if (!isSharedAlbum.value) return null;
  return props.currentAlbum.split("/")[0];
});

const currentAlbumCanWrite = computed(() => {
  if (!isSharedAlbum.value) return true;
  return currentAlbumObj.value?.can_write ?? false;
});

const currentAlbumCanDelete = computed(() => {
  if (!isSharedAlbum.value) return true;
  return currentAlbumObj.value?.can_delete ?? false;
});

const inDuplicatesView = computed(() => props.albumViewMode === 'duplicates');
const inSimilarView = computed(() => props.albumViewMode === 'similar');
const inTextView = computed(() => props.albumViewMode === 'text');
const inSpecialView = computed(() => inDuplicatesView.value || inSimilarView.value || inTextView.value);

watch(inSpecialView, (on) => {
  if (!on) return;
  filterPanelOpen.value = false;
  sortPanelOpen.value = false;
});

const textSearchInput = ref('');
const desktopSearchInputRef = ref(null);
/** Desktop toolbar search stays on the album grid. null means the album itself. */
const desktopSearchPhotos = ref(null);
const desktopSearchLoading = ref(false);
const desktopSearchError = ref("");
let desktopSearchToken = 0;

const showingDesktopSearch = computed(() => desktopSearchPhotos.value !== null);
const desktopSearchBar = computed(() => !inSpecialView.value && !isMobile.value);
/** Toolbar is too narrow for the full search field beside labeled actions. */
const desktopSearchCollapsed = ref(false);
/** User opened the field from the collapsed search button. */
const desktopSearchForcedOpen = ref(false);
const desktopSearchNeedWidth = ref(0);
const desktopToolbarRef = ref(null);
const desktopSearchFieldOpen = computed(() => desktopSearchBar.value && (
  !desktopSearchCollapsed.value
  || desktopSearchForcedOpen.value
  || showingDesktopSearch.value
  || desktopSearchLoading.value
));
const desktopToolbarIconsOnly = computed(() => (
  desktopSearchCollapsed.value
  && desktopSearchForcedOpen.value
  && !showingDesktopSearch.value
  && !desktopSearchLoading.value
));
const desktopActionLabelClass = computed(() => (
  desktopToolbarIconsOnly.value ? "hidden" : "hidden md:inline"
));
const gridPhotos = computed(() => (showingDesktopSearch.value ? desktopSearchPhotos.value : props.albumPhotos));

function openDesktopSearchField() {
  if (desktopSearchCollapsed.value) desktopSearchForcedOpen.value = true;
  nextTick(() => {
    const input = desktopSearchInputRef.value;
    if (!input) return;
    input.focus();
    input.select();
  });
}

function onDesktopSearchEscape() {
  if (!desktopSearchCollapsed.value || showingDesktopSearch.value || desktopSearchLoading.value) return;
  desktopSearchForcedOpen.value = false;
}

function measureDesktopToolbar() {
  const bar = desktopToolbarRef.value;
  if (!bar || !desktopSearchBar.value || props.embedded) {
    desktopSearchCollapsed.value = false;
    desktopSearchForcedOpen.value = false;
    return;
  }
  const width = bar.clientWidth;
  if (width <= 0) return;
  const showingFullField = desktopSearchFieldOpen.value
    && !desktopToolbarIconsOnly.value
    && !showingDesktopSearch.value
    && !desktopSearchLoading.value;
  if (showingFullField) {
    desktopSearchNeedWidth.value = bar.scrollWidth;
    const overflows = bar.scrollWidth > width + 1;
    desktopSearchCollapsed.value = overflows;
    if (!overflows) desktopSearchForcedOpen.value = false;
    return;
  }
  // Extra room avoids flipping back to the field on the same width that just overflowed.
  if (
    desktopSearchCollapsed.value
    && desktopSearchNeedWidth.value > 0
    && width >= desktopSearchNeedWidth.value + 16
  ) {
    desktopSearchCollapsed.value = false;
    desktopSearchForcedOpen.value = false;
    nextTick(measureDesktopToolbar);
  }
}

let desktopToolbarObserver = null;
watch(desktopToolbarRef, (bar) => {
  desktopToolbarObserver?.disconnect();
  desktopToolbarObserver = null;
  if (!bar || typeof ResizeObserver === "undefined") return;
  desktopToolbarObserver = new ResizeObserver(() => measureDesktopToolbar());
  desktopToolbarObserver.observe(bar);
  measureDesktopToolbar();
});
onBeforeUnmount(() => {
  desktopToolbarObserver?.disconnect();
  desktopToolbarObserver = null;
});

function resetDesktopSearch() {
  desktopSearchToken += 1;
  desktopSearchPhotos.value = null;
  desktopSearchLoading.value = false;
  desktopSearchError.value = "";
}

watch(() => props.currentAlbum, (album, previous) => {
  if (previous === undefined || album === previous) return;
  resetDesktopSearch();
  textSearchInput.value = '';
  desktopSearchForcedOpen.value = false;
  desktopSearchCollapsed.value = false;
  desktopSearchNeedWidth.value = 0;
});

watch(
  [desktopSearchBar, showingDesktopSearch, isSharedAlbum, currentAlbumCanDelete, inSpecialView, isMobile],
  () => nextTick(measureDesktopToolbar),
);

async function submitTextSearch() {
  const text = textSearchInput.value.trim();
  if (!text) {
    clearTextSearch();
    return;
  }
  if (!isMobile.value && !props.embedded && typeof props.onSearchTextInModal === 'function') {
    const token = ++desktopSearchToken;
    desktopSearchLoading.value = true;
    desktopSearchError.value = "";
    if (desktopSearchPhotos.value === null) desktopSearchPhotos.value = [];
    try {
      const data = await props.onSearchTextInModal({
        text,
        scope: 'current_album',
        limit: 10000,
      });
      if (token !== desktopSearchToken) return;
      desktopSearchPhotos.value = data?.results || [];
    } catch (error) {
      if (token !== desktopSearchToken) return;
      desktopSearchPhotos.value = [];
      desktopSearchError.value = error?.message || "";
    } finally {
      if (token === desktopSearchToken) desktopSearchLoading.value = false;
    }
    return;
  }
  emit('search-text', { text, scope: 'current_album' });
}

function clearTextSearch() {
  textSearchInput.value = '';
  if (showingDesktopSearch.value || desktopSearchLoading.value) {
    resetDesktopSearch();
    return;
  }
  emit('show-photos');
}

function onAlbumBack() {
  if (showingDesktopSearch.value || desktopSearchLoading.value) {
    clearTextSearch();
    return;
  }
  exitAlbum();
}

watch(showingDesktopSearch, (on) => {
  if (!on) return;
  gradeView.value = false;
  thumbnailPopoverOpen.value = false;
  viewSettingsOpen.value = false;
  selectMode.value = false;
  selectedPhotoPaths.value = new Set();
});

function toggleFilterSortPanel() {
  if (isMobile.value) {
    toggleFilterPanel();
    return;
  }
  filterSortPanelOpen.value = !filterSortPanelOpen.value;
}

function toggleFilterPanel() {
  sortPanelOpen.value = false;
  if (isMobile.value) {
    filterPanelOpen.value = !filterPanelOpen.value;
    return;
  }
  filterSortPanelOpen.value = !filterSortPanelOpen.value;
}

function toggleSortPanel() {
  filterPanelOpen.value = false;
  sortPanelOpen.value = !sortPanelOpen.value;
}

function closeFilterSortOverlays() {
  filterSortPanelOpen.value = false;
  filterPanelOpen.value = false;
  sortPanelOpen.value = false;
}

function toggleAiToolsPanel() {
  if (aiToolsPanelOpen.value) {
    aiToolsPanelOpen.value = false;
    aiToolsPickerOpen.value = false;
    return;
  }
  aiToolsPanelOpen.value = true;
  emit("refresh-album-people");
  if (!isMobile.value) return;
  closeFilterSortOverlays();
  mobileSearchOpen.value = false;
}

async function openMobileSearch() {
  mobileSearchOpen.value = true;
  await Promise.resolve();
  mobileSearchInputRef.value?.focus?.();
}

function closeMobileSearch() {
  mobileSearchOpen.value = false;
}

function enterAlbum(album) {
  albumHeroId.value = album.dir_name;
  runAlbumViewTransition(async () => {
    // Let the album-open animation paint before the grid fetch saturates the frame.
    const defer = albumEnterDeferMs();
    if (defer > 0) {
      await new Promise((resolve) => setTimeout(resolve, defer));
    }
    emit("select-album", { dirName: album.dir_name });
  });
}

function exitAlbum() {
  gradeView.value = false;
  gradePhoto.value = null;
  albumHeroId.value = props.currentAlbum;
  selectMode.value = false;
  selectedPhotoPaths.value.clear();
  runAlbumViewTransition(() => {
    emit("select-album", "");
  }).finally(() => {
    albumHeroId.value = "";
  });
}

function toggleSelectMode() {
  selectMode.value = !selectMode.value;
  if (!selectMode.value) selectedPhotoPaths.value.clear();
}

function togglePhotoSelect(photo) {
  const s = selectedPhotoPaths.value;
  if (s.has(photo.path)) s.delete(photo.path);
  else s.add(photo.path);
  // trigger reactivity
  selectedPhotoPaths.value = new Set(s);
}

function selectPhotoFromMenu(photo) {
  if (!photo?.path) return;
  if (!selectMode.value) {
    selectMode.value = true;
    selectedPhotoPaths.value = new Set([photo.path]);
    return;
  }
  const next = new Set(selectedPhotoPaths.value);
  next.add(photo.path);
  selectedPhotoPaths.value = next;
}

const menuPhoto = ref(null);
const menuHasXmp = ref(false);
const photoMenuOpen = ref(false);
const photoAlbumOpen = ref(false);
const suppressPhotoClick = ref(false);
let menuXmpToken = 0;
let photoPress = null;
let suppressPhotoClickTimer = 0;

function clearPhotoPress() {
  if (photoPress?.timer) window.clearTimeout(photoPress.timer);
  photoPress = null;
  window.removeEventListener("pointermove", onPhotoPressMove);
  window.removeEventListener("pointerup", clearPhotoPress);
  window.removeEventListener("pointercancel", clearPhotoPress);
}

function armSuppressPhotoClick() {
  suppressPhotoClick.value = true;
  window.clearTimeout(suppressPhotoClickTimer);
  suppressPhotoClickTimer = window.setTimeout(() => {
    suppressPhotoClick.value = false;
  }, 450);
}

function onPhotoPointerDown(photo, event) {
  if (!isMobile.value || !event.isPrimary || event.button > 0) return;
  clearPhotoPress();
  photoPress = {
    photo,
    x: event.clientX,
    y: event.clientY,
    timer: window.setTimeout(() => {
      const pressed = photoPress?.photo;
      clearPhotoPress();
      if (!pressed) return;
      armSuppressPhotoClick();
      openPhotoMenu(pressed);
    }, 450),
  };
  window.addEventListener("pointermove", onPhotoPressMove);
  window.addEventListener("pointerup", clearPhotoPress);
  window.addEventListener("pointercancel", clearPhotoPress);
}

function onPhotoPressMove(event) {
  if (!photoPress) return;
  const dx = event.clientX - photoPress.x;
  const dy = event.clientY - photoPress.y;
  if (dx * dx + dy * dy > 144) clearPhotoPress();
}

function onPhotoContextMenu(photo, event) {
  if (!isMobile.value) return;
  event.preventDefault();
  clearPhotoPress();
  armSuppressPhotoClick();
  openPhotoMenu(photo);
}

function onGridPhotoClick(photo) {
  if (suppressPhotoClick.value) {
    suppressPhotoClick.value = false;
    window.clearTimeout(suppressPhotoClickTimer);
    return;
  }
  if (selectMode.value) togglePhotoSelect(photo);
  else if (showingDesktopSearch.value) openPhotoDetail(photo, { navPhotos: desktopSearchPhotos.value });
  else openDetail(photo);
}

async function openPhotoMenu(photo) {
  menuPhoto.value = photo;
  menuHasXmp.value = false;
  photoMenuOpen.value = true;
  const token = ++menuXmpToken;
  try {
    const owner = photo.owner || currentAlbumOwner.value;
    const ownerParam = owner ? `?owner=${encodeURIComponent(owner)}` : "";
    const res = await httpFetch(`/api/albums/has_xmp/${encodePath(photo.path)}${ownerParam}`);
    if (!res.ok || token !== menuXmpToken || menuPhoto.value?.path !== photo.path) return;
    const data = await res.json();
    menuHasXmp.value = !!data.has_xmp;
  } catch {
    if (token === menuXmpToken) menuHasXmp.value = false;
  }
}

const photoMenuActions = computed(() => {
  const video = isVideoPhoto(menuPhoto.value);
  const items = [
    { id: "select", label: t("albums.select"), icon: IconCheck },
    { id: "download", label: t("detail.download"), icon: IconDownload },
  ];
  if (!video) {
    items.push({ id: "export", label: t("detail.exportBtn"), icon: IconDownload });
  }
  if (currentAlbumCanWrite.value && !video) {
    items.push({ id: "cover", label: t("detail.setAsCover"), icon: IconImage });
  }
  items.push(
    { id: "album", label: t("detail.addToAlbum"), icon: IconImage, disabled: addableAlbums.value.length === 0 },
  );
  if (!video) {
    items.push({ id: "similar", label: t("detail.findSimilar"), icon: IconSparkles });
  }
  if (currentAlbumCanDelete.value) {
    items.push({ id: "delete", label: t("detail.delete"), icon: IconTrash, danger: true });
  }
  return items;
});

const photoMenuAlbumActions = computed(() => addableAlbums.value.map((album) => ({
  id: album.dir_name,
  label: album.name || album.dir_name,
  subtitle: album.owner || "",
})));

function downloadPhotoFile(photo) {
  if (!photo?.path) return;
  // Blob download: a plain <a href> navigation hands the file to Safari's
  // inline viewer on iOS even with an attachment header.
  downloadFileByUrl(getOriginalUrl(photo.path, photo.owner), photo.name).catch((e) => {
    alert(e.message || "Download failed");
  });
}

function onPhotoMenuAction(action) {
  const photo = menuPhoto.value;
  photoMenuOpen.value = false;
  if (!photo || !action?.id) return;
  if (action.id === "select") {
    selectPhotoFromMenu(photo);
    menuPhoto.value = null;
  } else if (action.id === "download") {
    downloadPhotoFile(photo);
    menuPhoto.value = null;
  } else if (action.id === "export") {
    exportSelection.value = false;
    exportForm.value = { format: "jpeg", quality: 95, maxWidth: 0, applyXmp: menuHasXmp.value, applyGrade: false };
    showExportDialog.value = true;
  } else if (action.id === "cover") {
    setCoverFromPhoto(photo);
    menuPhoto.value = null;
  } else if (action.id === "album") {
    window.setTimeout(() => {
      if (menuPhoto.value?.path === photo.path) photoAlbumOpen.value = true;
    }, 280);
  } else if (action.id === "similar") {
    emit("find-similar", photo);
    menuPhoto.value = null;
  } else if (action.id === "delete") {
    if (!window.confirm(t("msg.confirmDeletePhoto", { name: photo.name }))) return;
    emit("delete-photos", [photo.path]);
    if (detailPhoto.value?.path === photo.path) {
      detailPhoto.value = null;
      detailExpanded.value = false;
    }
    menuPhoto.value = null;
  }
}

function onPhotoMenuAlbum(action) {
  const photo = menuPhoto.value;
  photoAlbumOpen.value = false;
  menuPhoto.value = null;
  if (!photo?.path || !action?.id) return;
  emit("copy-photo-to-album", {
    photoPath: photo.path,
    sourceAlbum: props.currentAlbum,
    targetAlbum: action.id,
  });
}

function closePhotoAlbumMenu() {
  photoAlbumOpen.value = false;
  menuPhoto.value = null;
}

onBeforeUnmount(() => {
  clearPhotoPress();
  window.clearTimeout(suppressPhotoClickTimer);
});

async function selectAllPhotos() {
  if (props.loadAllPhotos) {
    await props.loadAllPhotos();
  }
  selectedPhotoPaths.value = new Set(gridPhotos.value.map((p) => p.path));
}

function clearSelection() {
  selectedPhotoPaths.value = new Set();
  selectMode.value = false;
}

function confirmDeleteSelected() {
  if (!currentAlbumCanDelete.value) return;
  const count = selectedPhotoPaths.value.size;
  if (!count) return;
  if (!window.confirm(t('msg.confirmDeletePhotos', { count }))) return;
  emit("delete-photos", [...selectedPhotoPaths.value]);
  selectMode.value = false;
  selectedPhotoPaths.value = new Set();
}

const downloadSelectionPending = ref(false);

function albumFolderName(albumKey) {
  const raw = String(albumKey || "");
  const slash = raw.indexOf("/");
  return slash >= 0 ? raw.slice(slash + 1) : raw;
}

function selectedAlbumRequest() {
  const owner = currentAlbumOwner.value || props.currentUser;
  const albumname = albumFolderName(props.currentAlbum);
  const prefix = `albums/${albumname}/`;
  const photos = [];
  for (const path of selectedPhotoPaths.value) {
    if (!albumname || !String(path).startsWith(prefix)) return null;
    photos.push(String(path).slice(prefix.length));
  }
  if (!owner || photos.length === 0) return null;
  return { owner, albumname, photos };
}

function startZipDownload(url, filename) {
  // Same blob path as every other download: a plain anchor would open the ZIP in
  // Safari's inline viewer on iOS.
  downloadFileByUrl(url, filename).catch((error) => {
    alert(error.message || t("albums.downloadFailed"));
  });
}

async function downloadSelected() {
  if (downloadSelectionPending.value || selectedPhotoPaths.value.size === 0) return;
  const request = selectedAlbumRequest();
  if (!request) {
    alert(t("albums.downloadFailed"));
    return;
  }
  const { owner, albumname, photos } = request;
  downloadSelectionPending.value = true;
  try {
    const payload = await postJson("/api/albums/download", { owner, albumname, photos });
    if (!payload.data?.id) {
      throw new Error(t("albums.downloadFailed"));
    }
    startZipDownload(`/api/albums/download/${encodeURIComponent(payload.data.id)}`, `${albumname}.zip`);
  } catch (error) {
    alert(error.message || t("albums.downloadFailed"));
  } finally {
    downloadSelectionPending.value = false;
  }
}

function confirmDeleteSinglePhoto() {
  if (!detailPhoto.value) return;
  if (!window.confirm(t('msg.confirmDeletePhoto', { name: detailPhoto.value.name }))) return;
  emit("delete-photos", [detailPhoto.value.path]);
  detailPhoto.value = null;
  detailExpanded.value = false;
}

function getAlbumCoverThumbnail(album) {
  return albumCoverUrl(album, 400, currentAlbumOwner.value);
}

function duplicateDisplayName(photo) {
  return String(photo?.path || "").split("/").pop() || photo?.path || "";
}

function openDuplicateDetail(photo) {
  openDetail({
    ...photo,
    name: duplicateDisplayName(photo),
    datetime: "",
    date: "",
    tags: [],
    rating: 0,
    color_label: "",
    width: photo.width || 0,
    height: photo.height || 0,
  });
}

function findSimilarFromDetail() {
  if (!detailPhoto.value) return;
  const photo = detailPhoto.value;
  closeDetail();
  emit("find-similar", photo);
}

function openSimilarDetail(photo) {
  openDetail({
    ...photo,
    name: photo.name || duplicateDisplayName(photo),
    datetime: photo.datetime || photo.date || "",
    date: photo.date || "",
    tags: photo.tags || [],
    rating: photo.rating || 0,
    color_label: photo.color_label || [],
    width: photo.width || 0,
    height: photo.height || 0,
  });
}

// ── Filter helpers ──
const hasActiveFilters = computed(() => {
  const f = props.filters || {};
  return Array.isArray(f.clauses) && f.clauses.length > 0;
});

const hasActiveSort = computed(() => props.sort.length > 0);

function applyFilters() { emit("apply-filters"); }

const DUP_PREVIEW_LIMIT = 12;
const dupExpanded = ref({});

watch(
  () => props.duplicateGroups,
  () => {
    dupExpanded.value = {};
  },
);

function dupGroupKey(group, index = 0) {
  return group?.group_key || group?.id || `dup-${index}`;
}

function dupGroupPhotos(group, index = 0) {
  const photos = group?.photos || [];
  const key = dupGroupKey(group, index);
  if (dupExpanded.value[key] || photos.length <= DUP_PREVIEW_LIMIT) return photos;
  return photos.slice(0, DUP_PREVIEW_LIMIT);
}

function dupHiddenCount(group, index = 0) {
  const photos = group?.photos || [];
  const key = dupGroupKey(group, index);
  if (dupExpanded.value[key] || photos.length <= DUP_PREVIEW_LIMIT) return 0;
  return photos.length - DUP_PREVIEW_LIMIT;
}

function expandDupGroup(group, index = 0) {
  const key = dupGroupKey(group, index);
  dupExpanded.value = { ...dupExpanded.value, [key]: true };
}

// ── Photo grid / detail helpers ──
function handlePhotosScrollNearBottom() {
  if (showingDesktopSearch.value) return;
  emit("load-more-photos");
}

function getThumbnail(path, size = DEFAULT_THUMBNAIL_SIZE, owner = null) {
  return thumbnailUrl(path, { size, owner: owner || currentAlbumOwner.value });
}

function gridThumbOwner(photo) {
  return photo?.owner || currentAlbumOwner.value;
}

function getGridThumbnail(photo) {
  void gradedThumbEpoch.value;
  const owner = gridThumbOwner(photo);
  return thumbnailUrl(photo.path, {
    size: DEFAULT_THUMBNAIL_SIZE,
    owner,
    params: { graded: gradedThumbRev(owner, photo.path) },
  });
}

const detailPhotoList = computed(() => detailNavPhotos.value || props.albumPhotos);

const detailIndex = computed(() => {
  if (!detailPhoto.value) return -1;
  return detailPhotoList.value.findIndex((p) => p.path === detailPhoto.value.path);
});
const hasPrev = computed(() => detailIndex.value > 0);
const hasNext = computed(() => detailIndex.value >= 0 && detailIndex.value < detailPhotoList.value.length - 1);

const detailDirty = computed(() => {
  if (!detailPhoto.value) return false;
  const p = detailPhoto.value;
  if (editRating.value !== (p.rating || 0)) return true;
  const currentLabels = [...editColorLabels.value].sort().join(",");
  const originalLabels = colorLabels(p.color_label).sort().join(",");
  if (currentLabels !== originalLabels) return true;
  const currentTags = [...editTags.value].map((s) => s.trim()).filter(Boolean).sort().join(",");
  const originalTags = [...(p.tags || [])].map((s) => String(s).trim()).filter(Boolean).sort().join(",");
  if (currentTags !== originalTags) return true;
  return false;
});

function beginDetailImageLoad() {
  detailImageEpoch.value += 1;
  detailImageFallback.value = false;
  imageLoading.value = true;
}

function photoHasIndexMeta(photo) {
  if (!photo) return false;
  return Boolean(
    photo.datetime
    || photo.date
    || photo.camera_make
    || photo.camera_model
    || photo.lens
    || photo.focal_length
    || photo.aperture
    || photo.shutter_speed
    || photo.iso
    || (photo.size > 0 && photo.width > 0),
  );
}

function mergePhotoMeta(stub, indexed) {
  if (!stub) return null;
  if (!indexed) return stub;
  return {
    ...indexed,
    ...stub,
    path: stub.path || indexed.path,
    owner: stub.owner || indexed.owner,
    name: stub.name || indexed.name,
    datetime: indexed.datetime || indexed.date || stub.datetime || stub.date || "",
    date: indexed.date || stub.date || "",
    tags: indexed.tags ?? stub.tags ?? [],
    rating: indexed.rating ?? stub.rating ?? 0,
    color_label: indexed.color_label ?? stub.color_label ?? [],
    width: indexed.width || stub.width || 0,
    height: indexed.height || stub.height || 0,
    size: indexed.size ?? stub.size ?? 0,
    camera_make: indexed.camera_make || stub.camera_make || "",
    camera_model: indexed.camera_model || stub.camera_model || "",
    lens: indexed.lens || stub.lens || "",
    focal_length: indexed.focal_length || stub.focal_length || "",
    aperture: indexed.aperture || stub.aperture || "",
    shutter_speed: indexed.shutter_speed || stub.shutter_speed || "",
    iso: indexed.iso ?? stub.iso ?? null,
  };
}

async function enrichPhotosWithIndexMeta(stubs) {
  const list = (stubs || []).filter(Boolean);
  if (!list.length) return [];

  const byPath = new Map();
  for (const photo of props.albumPhotos) {
    if (photo?.path && photoHasIndexMeta(photo)) {
      byPath.set(photo.path, photo);
    }
  }

  const missingPaths = [...new Set(
    list
      .map((photo) => photo.path)
      .filter((path) => path && !byPath.has(path)),
  )];

  if (missingPaths.length > 0 && typeof props.fetchPhotosByPaths === "function") {
    try {
      const fetched = await props.fetchPhotosByPaths(missingPaths);
      for (const photo of fetched) {
        if (photo?.path) byPath.set(photo.path, photo);
      }
    } catch {
      // Fall back to stub-only metadata.
    }
  }

  return list.map((stub) => mergePhotoMeta(stub, byPath.get(stub.path)));
}

function withAlbumOwner(photo) {
  if (!photo) return photo;
  const owner = photo.owner || currentAlbumOwner.value;
  return owner && photo.owner !== owner ? { ...photo, owner } : photo;
}

function pauseDetailVideo() {
  detailVideoRef.value?.pause?.();
}

function openDetail(photo, expanded = detailOriginalPreference.value, options = {}) {
  pauseDetailVideo();
  detailRequestToken += 1;
  detailClosing.value = false;
  videoUnplayable.value = false;
  if (!options.fromNav) {
    detailNavPhotos.value = options.navPhotos ?? null;
    const fullscreen = options.fullscreen ?? detailFullscreenPreference.value;
    detailFullscreen.value = fullscreen;
    if (fullscreen) showFullscreenControls();
    else hideFullscreenControls();
  }
  beginDetailImageLoad();
  const next = withAlbumOwner(photo);
  detailPhoto.value = next;
  detailExpanded.value = expanded;
  detailDimensions.value = next.width > 0 && next.height > 0
    ? `${next.width} * ${next.height}`
    : null;
  editRating.value = next.rating || 0;
  editColorLabels.value = colorLabels(next.color_label);
  editTags.value = (next.tags || []).map((tag) => String(tag).trim()).filter(Boolean);
  // Restore darktable after XMP check; keep expanded preference immediately.
  darktablePreview.value = false;
  hasXmpSidecar.value = false;
  checkXmpSidecar(next.path, next.owner);
}

function openPhotoDetail(photo, options = {}) {
  if (!photo?.path) return;
  const { navPhotos = null, fullscreen, expanded } = options;
  void (async () => {
    const stubs = (navPhotos?.length ? navPhotos : [photo]).map((entry) => ({
      ...entry,
      name: entry.name || String(entry.path || "").split("/").pop() || entry.path,
    }));
    const enriched = await enrichPhotosWithIndexMeta(stubs);
    const main = enriched.find((entry) => entry.path === photo.path) || enriched[0];
    if (!main) return;
    openDetail(main, expanded ?? detailOriginalPreference.value, {
      navPhotos: enriched.length > 1 ? enriched : null,
      fullscreen,
    });
  })();
}

function onAiToolsOpenPhoto({ photo, navPhotos } = {}) {
  if (!photo?.path) return;
  void (async () => {
    const stubs = (navPhotos?.length ? navPhotos : [photo]).filter(Boolean);
    const enriched = await enrichPhotosWithIndexMeta(stubs);
    const main = enriched.find((entry) => entry.path === photo.path) || enriched[0];
    if (!main) return;
    openDetail(main, detailOriginalPreference.value, {
      navPhotos: enriched.length > 1 ? enriched : null,
    });
  })();
}

function closeDetail(options = {}) {
  pauseDetailVideo();
  const force = options === true || options.force;
  if (!force && !props.embedded && detailDirty.value && !window.confirm(t('msg.unsavedChangesClose'))) return;
  detailOriginalPreference.value = detailExpanded.value;
  detailDarktablePreference.value = darktablePreview.value;
  detailFullscreenPreference.value = detailFullscreen.value;
  saveDetailViewPreferences();
  detailRequestToken += 1;
  detailClosing.value = true;
  showExportDialog.value = false;
  if (props.embedded) finishCloseDetail();
}

function finishCloseDetail() {
  if (!detailClosing.value) return;
  detailPhoto.value = null;
  detailNavPhotos.value = null;
  detailExpanded.value = false;
  detailFullscreen.value = false;
  hideFullscreenControls();
  detailDimensions.value = null;
  detailImageFallback.value = false;
  darktablePreview.value = false;
  detailClosing.value = false;
}

function goDetailPrev() {
  if (!hasPrev.value) return;
  if (!props.embedded && detailDirty.value && !window.confirm(t('msg.unsavedChangesSwitch'))) return;
  openDetail(detailPhotoList.value[detailIndex.value - 1], detailExpanded.value, { fromNav: true });
}
function goDetailNext() {
  if (!hasNext.value) return;
  if (!props.embedded && detailDirty.value && !window.confirm(t('msg.unsavedChangesSwitch'))) return;
  openDetail(detailPhotoList.value[detailIndex.value + 1], detailExpanded.value, { fromNav: true });
}
function toggleExpanded() {
  beginDetailImageLoad();
  detailExpanded.value = !detailExpanded.value;
  detailOriginalPreference.value = detailExpanded.value;
  saveDetailViewPreferences();
}
function toggleDetailFullscreen() {
  beginDetailImageLoad();
  detailFullscreen.value = !detailFullscreen.value;
  detailFullscreenPreference.value = detailFullscreen.value;
  saveDetailViewPreferences();
  if (detailFullscreen.value) showFullscreenControls();
  else hideFullscreenControls();
}
let fullscreenControlsTimer = null;

function showFullscreenControls() {
  if (!detailFullscreen.value) return;
  detailFullscreenControlsVisible.value = true;
  if (fullscreenControlsTimer) clearTimeout(fullscreenControlsTimer);
  fullscreenControlsTimer = setTimeout(() => {
    detailFullscreenControlsVisible.value = false;
    fullscreenControlsTimer = null;
  }, 2000);
}

function hideFullscreenControls() {
  if (fullscreenControlsTimer) clearTimeout(fullscreenControlsTimer);
  fullscreenControlsTimer = null;
  detailFullscreenControlsVisible.value = false;
}

let lastDetailNavigationAt = 0;

function onDetailWheel(e) {
  if (!detailPhoto.value || e.deltaY === 0) return;
  const now = performance.now();
  if (now - lastDetailNavigationAt < 180) return;
  lastDetailNavigationAt = now;
  if (e.deltaY > 0) goDetailNext();
  else goDetailPrev();
}

function focusDesktopSearchFromShortcut(event) {
  if (event.defaultPrevented || event.altKey || event.shiftKey || event.isComposing) return false;
  if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "f") return false;
  if (props.embedded || props.gridPaused || !inAlbumView.value || !desktopSearchBar.value) return false;
  if (
    detailPhoto.value
    || showExportDialog.value
    || showAddToAlbumModal.value
    || showAlbumDialog.value
    || showShareDialog.value
    || aiToolsPanelOpen.value
    || viewSettingsOpen.value
    || importSourceModalOpen.value
    || cloudPickerOpen.value
  ) return false;
  const target = event.target;
  if (
    target instanceof HTMLElement
    && target !== desktopSearchInputRef.value
    && target.closest("input, textarea, select, [contenteditable='true']")
  ) return false;
  event.preventDefault();
  thumbnailPopoverOpen.value = false;
  openDesktopSearchField();
  return true;
}

function onDetailKeydown(e) {
  if (focusDesktopSearchFromShortcut(e)) return;
  if (e.altKey || e.ctrlKey || e.metaKey) return;
  if (e.key === "Escape") {
    if (e.defaultPrevented || props.embedded || !detailPhoto.value || detailClosing.value) return;
    e.preventDefault();
    if (showExportDialog.value) {
      showExportDialog.value = false;
      menuPhoto.value = null;
      return;
    }
    if (showAddToAlbumModal.value) {
      showAddToAlbumModal.value = false;
      return;
    }
    if (photoMenuOpen.value) {
      photoMenuOpen.value = false;
      return;
    }
    if (photoAlbumOpen.value) {
      closePhotoAlbumMenu();
      return;
    }
    closeDetail();
    return;
  }
  if (!detailPhoto.value) return;
  const target = e.target;
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return;
  if (e.key === "ArrowLeft") {
    e.preventDefault();
    goDetailPrev();
    return;
  }
  if (e.key === "ArrowRight") {
    e.preventDefault();
    goDetailNext();
    return;
  }
  // A bare letter otherwise starts the browser's page search, which walks the
  // virtual grid under the viewer and locks the tab.
  if (e.key.length === 1) e.preventDefault();
}
function toggleDarktablePreview() {
  if (!hasXmpSidecar.value) return;
  beginDetailImageLoad();
  darktablePreview.value = !darktablePreview.value;
  detailDarktablePreference.value = darktablePreview.value;
  saveDetailViewPreferences();
}
function onDetailImageLoad() {
  imageLoading.value = false;
  const photo = detailPhoto.value;
  if (photo?.path && String(detailImageSrc.value).includes("/api/albums/darktable_preview/")) {
    markGradedThumb(gridThumbOwner(photo), photo.path);
  }
}
function onDetailVideoReady(event) {
  const el = event?.target;
  if (!(el instanceof HTMLVideoElement) || !el.isConnected) return;
  imageLoading.value = false;
  if (el.videoWidth > 0 && el.videoHeight > 0) return;
  el.pause();
  videoUnplayable.value = true;
}
function onDetailVideoError(event) {
  const el = event?.target;
  if (el instanceof HTMLVideoElement && !el.isConnected) return;
  if (el instanceof HTMLVideoElement) el.pause();
  videoUnplayable.value = true;
  imageLoading.value = false;
}
function onDetailImageError() {
  // Fall back to thumbnail for display only; keep view-original / darktable toggles
  // so the user can retry. Remount via detailImageEpoch avoids aborted-load false errors.
  if (!detailImageFallback.value) {
    detailImageFallback.value = true;
    detailImageEpoch.value += 1;
    imageLoading.value = true;
    return;
  }
  imageLoading.value = false;
}
function getOriginalUrl(path, owner = null) {
  const resolvedOwner = owner ?? detailPhoto.value?.owner ?? currentAlbumOwner.value;
  const ownerParam = resolvedOwner ? `?owner=${encodeURIComponent(resolvedOwner)}` : "";
  return `/api/download/${encodePath(path)}${ownerParam}`;
}

function getDarktablePreviewUrl(path, size = 1600, owner = null) {
  const resolvedOwner = owner ?? detailPhoto.value?.owner ?? currentAlbumOwner.value;
  const ownerParam = resolvedOwner ? `&owner=${encodeURIComponent(resolvedOwner)}` : "";
  return `/api/albums/darktable_preview/${encodePath(path)}?size=${size}${ownerParam}`;
}

let detailRequestToken = 0;

async function checkXmpSidecar(path, owner = null) {
  const token = detailRequestToken;
  try {
    const resolvedOwner = owner ?? detailPhoto.value?.owner ?? currentAlbumOwner.value;
    const ownerParam = resolvedOwner ? `?owner=${encodeURIComponent(resolvedOwner)}` : "";
    const res = await httpFetch(`/api/albums/has_xmp/${encodePath(path)}${ownerParam}`);
    if (token !== detailRequestToken) return;
    if (res.ok) {
      const data = await res.json();
      if (token !== detailRequestToken || !detailPhoto.value || detailPhoto.value.path !== path) return;
      hasXmpSidecar.value = !!data.has_xmp;
      if (hasXmpSidecar.value) {
        const wantDarktable = detailDarktablePreference.value;
        if (wantDarktable !== darktablePreview.value) {
          beginDetailImageLoad();
          darktablePreview.value = wantDarktable;
        }
      }
    } else {
      hasXmpSidecar.value = false;
    }
  } catch {
    if (token === detailRequestToken) hasXmpSidecar.value = false;
  }
}

const RAW_EXTS = new Set(["cr2","cr3","nef","arw","dng","orf","rw2","raf","pef","srw"]);
function isRawFile(name) {
  const ext = (name || "").split(".").pop().toLowerCase();
  return RAW_EXTS.has(ext);
}
const isDetailVideo = computed(() => isVideoPhoto(detailPhoto.value));
const detailDurationLabel = computed(() => formatDurationMs(detailPhoto.value?.duration_ms));
const detailVideoCanPlay = computed(() => {
  const mode = detailVideoPlan.value?.mode;
  return mode === "original" || mode === "encode";
});
const detailPlaybackSrc = computed(() => {
  if (!detailPhoto.value) return "";
  if (detailVideoPlan.value?.mode === "encode" && detailVideoPlan.value.url) {
    return detailVideoPlan.value.url;
  }
  const owner = detailPhoto.value.owner || currentAlbumOwner.value;
  return playbackUrl(detailPhoto.value.path, owner);
});
const detailImageSrc = computed(() => {
  if (!detailPhoto.value) return "";
  if (isDetailVideo.value) {
    return getThumbnail(detailPhoto.value.path, 800, detailPhoto.value.owner);
  }
  const isRaw = isRawFile(detailPhoto.value.name);

  // Explicit darktable toggle (requires XMP): always use darktable_preview.
  if (darktablePreview.value && hasXmpSidecar.value) {
    const size = detailFullscreen.value || detailExpanded.value || isMobile.value ? 0 : 1600;
    return getDarktablePreviewUrl(detailPhoto.value.path, size);
  }

  if (detailImageFallback.value) {
    return getThumbnail(detailPhoto.value.path, 800, detailPhoto.value.owner);
  }

  // Non-darktable: JPEG/etc can use original; RAW goes through /api/thumbnail (LibRaw/lumeno).
  // size=0 requests full-resolution decode (same as darktable_preview).
  if (detailFullscreen.value || detailExpanded.value) {
    if (isRaw) {
      return getThumbnail(detailPhoto.value.path, 0, detailPhoto.value.owner);
    }
    return getOriginalUrl(detailPhoto.value.path);
  }

  return getThumbnail(detailPhoto.value.path, isMobile.value ? 1600 : 1200, detailPhoto.value.owner);
});

function downloadOriginal() {
  const photo = detailPhoto.value;
  if (!photo) return;
  // Blob download: a plain <a href> navigation hands the file to Safari's
  // inline viewer on iOS even with an attachment header.
  downloadFileByUrl(getOriginalUrl(photo.path), photo.name).catch((e) => {
    alert(e.message || "Download failed");
  });
}

function openExportDialog() {
  exportSelection.value = false;
  exportForm.value = { format: "jpeg", quality: 95, maxWidth: 0, applyXmp: hasXmpSidecar.value, applyGrade: false };
  showExportDialog.value = true;
}

// The two grading switches are alternatives: turning one on turns the other off.
function onToggleApplyXmp() {
  if (exportForm.value.applyXmp) exportForm.value.applyGrade = false;
}
function onToggleApplyGrade() {
  if (exportForm.value.applyGrade) exportForm.value.applyXmp = false;
}

function openExportSelection() {
  if (exportSelectionPending.value || selectedPhotoPaths.value.size === 0) return;
  menuPhoto.value = null;
  exportSelection.value = true;
  exportForm.value = { format: "jpeg", quality: 95, maxWidth: 0, applyXmp: true, applyGrade: false };
  showExportDialog.value = true;
}

async function exportSelected() {
  if (exportSelectionPending.value || exporting.value) return;
  const request = selectedAlbumRequest();
  if (!request) {
    alert(t("albums.exportFailed"));
    return;
  }
  const { owner, albumname, photos } = request;
  exportSelectionPending.value = true;
  exporting.value = true;
  try {
    const payload = await postJson("/api/albums/export_batch", {
      owner,
      albumname,
      photos,
      format: exportForm.value.format,
      quality: Number(exportForm.value.quality) || 95,
      max_width: Number(exportForm.value.maxWidth) || 0,
      apply_xmp: !!exportForm.value.applyXmp,
      apply_grade: !!exportForm.value.applyGrade,
    });
    if (!payload.data?.id) {
      throw new Error(t("albums.exportFailed"));
    }
    startZipDownload(`/api/albums/export_batch/${encodeURIComponent(payload.data.id)}`, `${albumname}.zip`);
    showExportDialog.value = false;
    exportSelection.value = false;
  } catch (error) {
    alert(error.message || t("albums.exportFailed"));
  } finally {
    exportSelectionPending.value = false;
    exporting.value = false;
  }
}

async function doExport() {
  if (exportSelection.value) {
    await exportSelected();
    return;
  }
  const photo = menuPhoto.value || detailPhoto.value;
  if (!photo || exporting.value) return;
  exporting.value = true;
  try {
    const body = {
      path: photo.path,
      format: exportForm.value.format,
      quality: exportForm.value.quality,
      max_width: exportForm.value.maxWidth || 0,
      apply_xmp: !!exportForm.value.applyXmp,
      apply_grade: !!exportForm.value.applyGrade,
    };
    if (photo.owner || currentAlbumOwner.value) {
      body.owner = photo.owner || currentAlbumOwner.value;
    }
    // The success body is the file itself; `httpJson` passes the response through
    // and throws the server's message for a JSON error.
    const res = await httpJson(
      "/api/albums/export",
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      },
      t("albums.exportFailed"),
    );
    await saveBlobResponse(res, `export.${exportForm.value.format}`);
    showExportDialog.value = false;
    menuPhoto.value = null;
  } catch (e) {
    alert(e.message || "Export failed");
  } finally {
    exporting.value = false;
  }
}

function setDetailRating(star) { editRating.value = editRating.value === star ? 0 : star; }
function setDetailColorLabel(key) {
  const idx = editColorLabels.value.indexOf(key);
  if (idx >= 0) editColorLabels.value.splice(idx, 1);
  else editColorLabels.value.push(key);
}

function savePhotoMeta() {
  if (!detailPhoto.value) return;
  const tags = [...editTags.value].map((s) => s.trim()).filter(Boolean);
  emit("update-photo-meta", {
    path: detailPhoto.value.path,
    album: detailPhoto.value.album,
    tags,
    rating: editRating.value,
    colorLabel: [...editColorLabels.value],
  });
  detailPhoto.value.rating = editRating.value;
  detailPhoto.value.color_label = [...editColorLabels.value];
  detailPhoto.value.tags = tags;
  closeDetail();
}

function gradeThumb(photo) {
  if (!photo?.path) return "";
  return getGridThumbnail(photo);
}

function pickGradePhoto() {
  if (detailPhoto.value && !isVideoPhoto(detailPhoto.value)) return detailPhoto.value;
  return gridPhotos.value.find((photo) => photo?.path && !isVideoPhoto(photo)) || null;
}

async function useGradePhoto(photo) {
  if (!photo?.path) {
    gradePhoto.value = null;
    return;
  }
  const token = ++gradeEnrichToken;
  const stub = withAlbumOwner(photo);
  gradePhoto.value = stub;
  const enriched = await enrichPhotosWithIndexMeta([stub]);
  if (token !== gradeEnrichToken) return;
  const next = enriched.find((entry) => entry.path === stub.path) || enriched[0];
  if (!next || gradePhoto.value?.path !== next.path) return;
  gradePhoto.value = withAlbumOwner(next);
}

function toggleGradeView() {
  thumbnailPopoverOpen.value = false;
  if (gradeView.value) {
    gradeView.value = false;
    return;
  }
  if (isMobile.value) return;
  const photo = pickGradePhoto();
  if (!photo) {
    alert(t("grade.noPhoto"));
    return;
  }
  if (detailPhoto.value) {
    closeDetail();
    if (detailPhoto.value && !detailClosing.value) return;
  }
  gradeView.value = true;
  void useGradePhoto(photo);
}

function onGradeSelect(photo) {
  void useGradePhoto(photo);
}

function downloadGradePhoto(photo) {
  if (!photo) return;
  downloadFileByUrl(getOriginalUrl(photo.path, photo.owner), photo.name).catch((error) => {
    alert(error.message || "Download failed");
  });
}

async function exportGradePhoto(photo) {
  if (!photo) return;
  menuPhoto.value = photo;
  let hasXmp = false;
  try {
    const owner = photo.owner || currentAlbumOwner.value;
    const ownerParam = owner ? `?owner=${encodeURIComponent(owner)}` : "";
    const res = await httpFetch(`/api/albums/has_xmp/${encodePath(photo.path)}${ownerParam}`);
    if (res.ok) {
      const data = await res.json();
      hasXmp = !!data.has_xmp;
    }
  } catch {
    hasXmp = false;
  }
  menuHasXmp.value = hasXmp;
  exportSelection.value = false;
  exportForm.value = { format: "jpeg", quality: 95, maxWidth: 0, applyXmp: false, applyGrade: true };
  showExportDialog.value = true;
}

function saveGradeMeta(payload) {
  emit("update-photo-meta", payload);
  const apply = (photo) => {
    if (!photo || photo.path !== payload.path) return;
    photo.rating = payload.rating;
    photo.color_label = payload.colorLabel;
    photo.tags = payload.tags;
  };
  apply(gradePhoto.value);
  apply(props.albumPhotos.find((photo) => photo.path === payload.path));
}

function photoColorDot(label) { return COLOR_LABELS.find((c) => c.key === label); }

// ── Add to album ────────────────────────────────────────────────────────────

// All albums (own + shared-with-write) except the current one
const addableAlbums = computed(() => {
  const all = [
    ...props.albums.map((a) => ({ ...a, canWrite: true, owner: null })),
    ...props.sharedAlbums
      .filter((a) => a.can_write)
      .map((a) => ({ ...a, canWrite: true })),
  ];
  return all.filter((a) => a.dir_name !== props.currentAlbum);
});

function openAddToAlbumModal(photo) {
  addToAlbumPhoto.value = photo;
  showAddToAlbumModal.value = true;
}

async function doAddToAlbum(targetAlbum) {
  if (!addToAlbumPhoto.value || addToAlbumLoading.value) return;
  addToAlbumLoading.value = true;
  try {
    emit("copy-photo-to-album", {
      photoPath: addToAlbumPhoto.value.path,
      sourceAlbum: props.currentAlbum,
      targetAlbum: targetAlbum.dir_name,
    });
    showAddToAlbumModal.value = false;
  } finally {
    addToAlbumLoading.value = false;
  }
}

function getAddableAlbumCover(album) {
  return albumCoverUrl(album, 160);
}

function setCoverFromPhoto(photo) {
  if (!currentAlbumObj.value || !currentAlbumCanWrite.value || !photo?.path) return;
  void props.onUpdateAlbum({
    dirName: currentAlbumObj.value.dir_name,
    cover: photo.path,
  });
}

defineExpose({
  openPhotoDetail,
  openAiToolsPhoto: onAiToolsOpenPhoto,
  closeDetail,
  openImportModal,
  toggleSelectMode,
  toggleFilterSortPanel,
  toggleFilterPanel,
  toggleSortPanel,
  toggleAiToolsPanel,
  openViewSettings,
  viewLayout,
  setViewLayout,
  openMobileSearch,
  closeMobileSearch,
  submitTextSearch,
  openCreateDialog,
  openEditDialog,
  openShareDialog,
  confirmDeleteAlbum,
  textSearchInput,
  selectMode,
  selectedPhotoCount: computed(() => selectedPhotoPaths.value.size),
  selectAllPhotos,
  clearSelection,
  confirmDeleteSelected,
  downloadSelected,
  downloadSelectionPending,
  openExportSelection,
  exportSelectionPending,
  filterSortPanelOpen,
  filterPanelOpen,
  sortPanelOpen,
  aiToolsPanelOpen,
  aiToolsPickerOpen,
  hasActiveFilters,
  hasActiveSort,
  inSpecialView,
  isSharedAlbum,
  currentAlbumCanDelete,
  currentAlbumCanWrite,
  currentAlbumObj,
  detailPhoto,
  detailIndex,
  detailPhotoList,
  goDetailPrev,
  goDetailNext,
  hasPrev,
  hasNext,
  thumbnailSize,
  setThumbnailSize(value) {
    thumbnailSize.value = Number(value);
  },
  setViewCompact,
  viewCompact: computed(() => !!viewLayout.value.compact),
  forcedThumbsOnly,
  resetThumbnailSize() {
    thumbnailSize.value = THUMB_DEFAULT;
  },
});
</script>

<template>
  <section
    class="flex h-full min-h-0 flex-col overflow-hidden"
    :class="[
      isDark ? 'text-stone-100' : '',
      embedded ? '' : 'app-frost-glass',
      embedded || isMobile ? 'rounded-none' : 'app-card rounded-2xl',
    ]"
  >
    <!-- ═══════════════════════════════════════════
         VIEW 1: Album List
         ═══════════════════════════════════════════ -->
    <component
      :is="canAlbumVt ? 'div' : Transition"
      v-bind="canAlbumVt ? { class: 'contents' } : { name: 'album-view', mode: 'out-in' }"
    >
    <div v-if="!embedded && !inAlbumView" key="album-list" class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- Toolbar -->
      <div class="album-toolbar relative z-10 flex shrink-0 items-center gap-1.5 app-split-b px-2.5 py-2.5 app-frost-toolbar md:gap-2 md:px-4 md:py-3">
        <h2 class="text-sm font-semibold font-display" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t('albums.title') }}</h2>
        <div class="flex-1"></div>
        <button class="app-btn app-btn-primary px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          @click="openCreateDialog">
          <IconPlus class="h-3.5 w-3.5" /><span class="hidden md:inline">{{ t('albums.create') }}</span>
        </button>
        <button class="app-btn app-btn-secondary px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          @click="emit('refresh-albums')">
          <IconRefresh class="h-3.5 w-3.5" /><span class="hidden md:inline">{{ t('albums.refresh') }}</span>
        </button>
      </div>

      <!-- Album grid -->
      <div class="album-no-drag app-scrollbar min-h-0 flex-1 overflow-y-auto p-3 md:p-5" @mousedown="preventAlbumDragLock" @dragstart.prevent>
        <div v-if="busyAlbums && albums.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loading') }}</div>
        <div v-else-if="albums.length === 0" class="flex flex-col items-center justify-center gap-3 py-16">
          <IconImage class="h-12 w-12" :class="isDark ? 'text-stone-600' : 'text-stone-300'" />
          <p class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.empty') }}</p>
        </div>
        <div v-else class="grid grid-cols-1 gap-3 sm:grid-cols-2 md:gap-4 lg:grid-cols-3 xl:grid-cols-4">
          <article v-for="album in albums" :key="album.dir_name"
            class="group cursor-pointer overflow-hidden rounded-xl app-surface app-card transition"
            @click="enterAlbum(album)">
            <!-- Cover area -->
            <div
              class="relative aspect-[16/10] overflow-hidden"
              :class="isDark ? 'bg-stone-800' : 'bg-stone-100'"
              :style="albumHeroStyle(album.dir_name)"
            >
              <img v-if="album.cover"
                class="h-full w-full object-cover transition group-hover:scale-105"
                :src="getAlbumCoverThumbnail(album)" loading="lazy" decoding="async" alt="" />
              <div v-else class="flex h-full w-full items-center justify-center">
                <IconImage class="h-10 w-10" :class="isDark ? 'text-stone-600' : 'text-stone-300'" />
              </div>
              <!-- Photo count badge -->
              <span class="absolute bottom-2 right-2 rounded-full px-2 py-0.5 text-[11px] font-medium backdrop-blur"
                :class="isDark ? 'bg-black/50 text-stone-200' : 'bg-white/70 text-stone-700'">
                {{ t('albums.photoCount', { count: album.photo_count }) }}
              </span>
            </div>
            <!-- Info -->
            <div class="flex items-start gap-2 px-3 py-2.5">
              <div class="min-w-0 flex-1">
                <p class="truncate text-sm font-semibold" :title="album.name">{{ album.name || album.dir_name }}</p>
                <p v-if="album.description" class="mt-0.5 line-clamp-2 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ album.description }}</p>
              </div>
              <div class="flex shrink-0 gap-1" @click.stop>
                <button class="rounded-lg p-1.5 transition"
                  :class="isDark ? 'text-stone-400 hover:bg-stone-700 hover:text-stone-200' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-700'"
                  :title="t('albums.share')" @click="openShareDialog(album)">
                  <svg class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M15 19l-7-7 7-7"/><circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/></svg>
                </button>
                <button class="rounded-lg p-1.5 transition"
                  :class="isDark ? 'text-stone-400 hover:bg-stone-700 hover:text-stone-200' : 'text-stone-400 hover:bg-stone-200 hover:text-stone-700'"
                  :title="t('albums.edit')" @click="openEditDialog(album)">
                  <IconPencil class="h-3.5 w-3.5" />
                </button>
                <button class="rounded-lg p-1.5 transition"
                  :class="isDark ? 'text-stone-400 hover:bg-rose-900/40 hover:text-rose-300' : 'text-stone-400 hover:bg-rose-100 hover:text-rose-600'"
                  :title="t('albums.deleteAlbum')" @click="confirmDeleteAlbum(album)">
                  <IconTrash class="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
            <!-- Sharing badges -->
            <div v-if="album.shares && album.shares.length" class="flex flex-wrap gap-1 px-3 pb-2">
              <span v-for="s in album.shares" :key="s.username"
                class="inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium"
                :class="isDark ? 'bg-blue-900/40 text-blue-300' : 'bg-blue-100 text-blue-700'">
                {{ s.username }}
                <span v-if="s.can_write" class="opacity-60">{{ t('albums.write') }}</span>
                <span v-if="s.can_delete" class="opacity-60">{{ t('albums.deletePermission') }}</span>
              </span>
            </div>
          </article>
        </div>

        <!-- Shared albums section -->
        <template v-if="sharedAlbums.length > 0">
          <h3 class="mt-6 mb-3 text-xs font-semibold uppercase tracking-wide"
            :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.sharedAlbums') }}</h3>
          <div class="grid grid-cols-1 gap-3 sm:grid-cols-2 md:gap-4 lg:grid-cols-3 xl:grid-cols-4">
            <article v-for="album in sharedAlbums" :key="album.dir_name"
              class="group cursor-pointer overflow-hidden rounded-xl app-surface app-card transition"
              @click="enterAlbum(album)">
              <div
                class="relative aspect-[16/10] overflow-hidden"
                :class="isDark ? 'bg-stone-800' : 'bg-stone-100'"
                :style="albumHeroStyle(album.dir_name)"
              >
                <img v-if="album.cover"
                  class="h-full w-full object-cover transition group-hover:scale-105"
                  :src="getAlbumCoverThumbnail(album)" loading="lazy" decoding="async" alt="" />
                <div v-else class="flex h-full w-full items-center justify-center">
                  <IconImage class="h-10 w-10" :class="isDark ? 'text-stone-600' : 'text-stone-300'" />
                </div>
                <span class="absolute bottom-2 right-2 rounded-full px-2 py-0.5 text-[11px] font-medium backdrop-blur"
                  :class="isDark ? 'bg-black/50 text-stone-200' : 'bg-white/70 text-stone-700'">
                  {{ t('albums.photoCount', { count: album.photo_count }) }}
                </span>
                <span class="absolute top-2 left-2 rounded-full px-2 py-0.5 text-[10px] font-medium backdrop-blur"
                  :class="isDark ? 'bg-blue-900/60 text-blue-200' : 'bg-blue-100/80 text-blue-700'">
                  {{ album.owner }}
                </span>
              </div>
              <div class="px-3 py-2.5">
                <p class="truncate text-sm font-semibold" :title="album.name">{{ album.name || album.dir_name }}</p>
                <p class="mt-0.5 text-xs" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  {{ t('albums.from', { owner: album.owner }) }}
                  <span v-if="album.can_write" class="ml-1">· {{ t('albums.canEdit') }}</span>
                  <span v-if="album.can_delete" class="ml-1">· {{ t('albums.canDelete') }}</span>
                </p>
              </div>
            </article>
          </div>
        </template>
      </div>
    </div>

    <!-- ═══════════════════════════════════════════
         VIEW 2: Photo Browser (inside an album)
         ═══════════════════════════════════════════ -->
    <div v-else key="photo-browser" class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div class="relative z-10 flex shrink-0 flex-col">
      <!-- Top toolbar (desktop / non-embedded mobile). Phone shell supplies its own chrome. -->
      <div v-if="!embedded" ref="desktopToolbarRef" class="album-toolbar flex min-w-0 items-center gap-1.5 overflow-x-hidden app-split-b px-2.5 py-2.5 app-frost-toolbar md:gap-2 md:px-4 md:py-3">
        <template v-if="gradeView">
          <button class="app-btn app-btn-secondary shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
            @click="gradeView = false">
            <IconArrowLeft class="h-3.5 w-3.5" /><span class="hidden md:inline">{{ t('view.gradeBack') }}</span>
          </button>
          <div class="min-w-0 flex-1"></div>
          <button v-if="!isSharedAlbum" class="app-btn app-btn-secondary shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
            @click="resourcesOpen = true">
            {{ t('grade.resources') }}
          </button>
        </template>
        <template v-else>
        <div class="flex w-0 min-w-min flex-1 items-center gap-1.5 md:gap-2">
        <button class="app-btn app-btn-secondary shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          @click="onAlbumBack">
          <IconArrowLeft class="h-3.5 w-3.5" /><span class="hidden md:inline">{{ t('albums.back') }}</span>
        </button>
        <div
          class="relative h-10 w-14 shrink-0 overflow-hidden rounded-lg md:h-8 md:w-12"
          :class="isDark ? 'bg-stone-800' : 'bg-stone-200'"
          :style="{ viewTransitionName: 'album-hero' }"
        >
          <img
            v-if="currentAlbumObj?.cover"
            class="h-full w-full object-cover"
            :src="getAlbumCoverThumbnail(currentAlbumObj)"
            alt=""
          />
          <div v-else class="flex h-full w-full items-center justify-center">
            <IconImage class="h-4 w-4" :class="isDark ? 'text-stone-500' : 'text-stone-400'" />
          </div>
        </div>
        <h2 class="w-0 min-w-0 flex-1 truncate text-sm font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
          {{ currentAlbumObj?.name || currentAlbum }}
          <span v-if="isSharedAlbum" class="ml-1 text-xs font-normal" :class="isDark ? 'text-blue-400' : 'text-blue-600'">
            (​{{ t('albums.from', { owner: currentAlbumOwner }) }})
          </span>
        </h2>
        </div>
        <!-- Desktop: full search field when it fits, or after the collapsed button is opened. -->
        <div v-if="desktopSearchFieldOpen" class="flex items-center gap-1"
          :class="desktopToolbarIconsOnly ? 'min-w-32 max-w-[28rem] flex-1' : 'w-[28rem] min-w-32 max-w-[28rem] shrink'">
          <input
            ref="desktopSearchInputRef"
            v-model="textSearchInput"
            type="text"
            autocomplete="off"
            :placeholder="t('albums.textSearchPlaceholder')"
            class="min-w-0 flex-1 rounded-xl border px-2.5 py-1.5 text-xs"
            :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
            @keydown.enter.prevent="submitTextSearch"
            @keydown.escape.prevent="onDesktopSearchEscape"
          />
          <button
            class="inline-flex shrink-0 items-center rounded-xl px-2 py-1.5 text-xs font-medium transition"
            :class="isDark ? 'bg-amber-600 text-white hover:bg-amber-500' : 'bg-amber-600 text-white hover:bg-amber-700'"
            :title="t('albums.textSearch')"
            @click="submitTextSearch"
          >
            <IconSearch class="h-3.5 w-3.5" />
          </button>
          <button
            v-if="textSearchInput || showingDesktopSearch"
            class="inline-flex shrink-0 items-center rounded-xl px-2 py-1.5 text-xs transition"
            :class="isDark ? 'bg-stone-800 text-stone-300 hover:bg-stone-700' : 'bg-stone-200 text-stone-600 hover:bg-stone-300'"
            :title="t('albums.clear')"
            @click="clearTextSearch"
          >
            <IconClose class="h-3.5 w-3.5" />
          </button>
        </div>
        <div class="flex items-center justify-end gap-1.5 whitespace-nowrap md:gap-2"
          :class="desktopSearchFieldOpen && !desktopToolbarIconsOnly ? 'min-w-max flex-1' : (desktopSearchBar ? 'shrink-0' : 'min-w-0 flex-1')">
        <button
          v-if="desktopSearchBar && !desktopSearchFieldOpen"
          class="app-btn app-btn-secondary shrink-0 px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :title="t('albums.textSearch')"
          @click="openDesktopSearchField"
        >
          <IconSearch class="h-3.5 w-3.5" />
        </button>
        <!-- Mobile: collapsed search button / expanded field -->
        <template v-if="!inSpecialView && isMobile">
          <button
            v-if="!mobileSearchOpen"
            class="inline-flex items-center rounded-xl px-2.5 py-1.5 text-xs font-medium transition"
            :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
            :title="t('albums.textSearch')"
            @click="openMobileSearch"
          >
            <IconSearch class="h-3.5 w-3.5" />
          </button>
          <div v-else class="flex min-w-0 flex-1 items-center gap-1">
            <input
              ref="mobileSearchInputRef"
              v-model="textSearchInput"
              type="text"
              autocomplete="off"
              :placeholder="t('albums.textSearchPlaceholder')"
              class="min-w-0 flex-1 rounded-xl border px-2.5 py-1.5 text-xs"
              :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
              @keydown.enter.prevent="submitTextSearch"
              @keydown.escape.prevent="closeMobileSearch"
            />
            <button
              class="inline-flex shrink-0 items-center rounded-xl px-2 py-1.5 text-xs font-medium transition"
              :class="isDark ? 'bg-amber-600 text-white hover:bg-amber-500' : 'bg-amber-600 text-white hover:bg-amber-700'"
              :title="t('albums.textSearch')"
              @click="submitTextSearch"
            >
              <IconSearch class="h-3.5 w-3.5" />
            </button>
            <button
              class="inline-flex shrink-0 items-center rounded-xl px-2 py-1.5 text-xs transition"
              :class="isDark ? 'bg-stone-800 text-stone-300 hover:bg-stone-700' : 'bg-stone-200 text-stone-600 hover:bg-stone-300'"
              :title="t('albums.clear')"
              @click="clearTextSearch(); closeMobileSearch()"
            >
              <IconClose class="h-3.5 w-3.5" />
            </button>
          </div>
        </template>
        <button v-if="!isSharedAlbum && !inSpecialView && !showingDesktopSearch && !(isMobile && mobileSearchOpen)" class="inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-2xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="isDark ? 'bg-emerald-700 text-white hover:bg-emerald-600' : 'bg-emerald-600 text-white hover:bg-emerald-700'"
          :title="t('albums.import')"
          @click="openImportModal">
          <IconImport class="h-3.5 w-3.5" /><span :class="desktopActionLabelClass">{{ t('albums.import') }}</span>
        </button>
        <button v-if="!inSpecialView && !showingDesktopSearch && !(isMobile && mobileSearchOpen)" class="app-btn shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :class="selectMode ? 'app-btn-accent' : 'app-btn-secondary'"
          :title="t('albums.select')"
          @click="toggleSelectMode">
          <IconCheck class="h-3.5 w-3.5" /><span :class="desktopActionLabelClass">{{ t('albums.select') }}</span>
        </button>
        <button v-if="!showingDesktopSearch && !(isMobile && mobileSearchOpen)" class="app-btn app-btn-secondary shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :title="t('albums.rebuildIndex')"
          @click="emit('rebuild-index')">
          <IconDatabase class="h-3.5 w-3.5" /><span :class="desktopActionLabelClass">{{ t('albums.rebuildIndex') }}</span>
        </button>
        <button
          v-if="inSpecialView"
          class="app-btn app-btn-accent shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :title="t('albums.backToPhotos')"
          @click="emit('show-photos')"
        >
          <IconImage class="h-3.5 w-3.5" />
          <span :class="desktopActionLabelClass">{{ t('albums.backToPhotos') }}</span>
        </button>
        <button v-if="!inSpecialView && !showingDesktopSearch && !isMobile" class="app-btn shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :class="filterSortPanelOpen ? 'app-btn-accent' : 'app-btn-secondary'"
          :title="t('albums.filters')"
          @click="toggleFilterSortPanel">
          <IconFilter class="h-3.5 w-3.5" />
          <span :class="desktopActionLabelClass">{{ t('albums.filters') }}</span>
          <span v-if="hasActiveFilters" class="ml-0.5 h-1.5 w-1.5 rounded-full bg-amber-500"></span>
        </button>
        <button v-if="!inSpecialView && !showingDesktopSearch && !isMobile" class="app-btn app-btn-secondary shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :title="t('view.grade')"
          @click="toggleGradeView">
          <IconGrade class="h-3.5 w-3.5" />
          <span :class="desktopActionLabelClass">{{ t('view.grade') }}</span>
        </button>
        <button v-if="!inSpecialView && !showingDesktopSearch && isMobile && !mobileSearchOpen" class="app-btn px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :class="filterPanelOpen ? 'app-btn-accent' : 'app-btn-secondary'"
          :title="t('albums.filters')"
          @click="toggleFilterPanel">
          <IconFilter class="h-3.5 w-3.5" />
          <span v-if="hasActiveFilters" class="ml-0.5 h-1.5 w-1.5 rounded-full bg-amber-500"></span>
        </button>
        <button v-if="!inSpecialView && !showingDesktopSearch && isMobile && !mobileSearchOpen" class="app-btn px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :class="sortPanelOpen ? 'app-btn-accent' : 'app-btn-secondary'"
          :title="t('albums.sort')"
          @click="toggleSortPanel">
          <IconSort class="h-3.5 w-3.5" />
          <span v-if="hasActiveSort" class="ml-0.5 h-1.5 w-1.5 rounded-full bg-amber-500"></span>
        </button>
        <button v-if="!showingDesktopSearch && !(isMobile && mobileSearchOpen)" class="app-btn shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
          :class="aiToolsPanelOpen || inSpecialView ? 'app-btn-accent' : 'app-btn-secondary'"
          :title="t('albums.aiTools')"
          @click="toggleAiToolsPanel">
          <IconWand class="h-3.5 w-3.5" />
          <span :class="desktopActionLabelClass">{{ t('albums.aiTools') }}</span>
        </button>
        <div v-if="!inSpecialView && !showingDesktopSearch && !(isMobile && mobileSearchOpen)" class="relative shrink-0">
          <button ref="thumbnailBtnRef" class="app-btn shrink-0 whitespace-nowrap px-2.5 py-1.5 text-xs md:px-3 md:py-2"
            :class="thumbnailPopoverOpen || viewSettingsOpen ? 'app-btn-accent' : 'app-btn-secondary'"
            :title="t('view.settings')"
            @click="toggleThumbnailPopover">
            <IconGrid class="h-3.5 w-3.5" />
            <span :class="desktopActionLabelClass">{{ t('view.settings') }}</span>
          </button>
          <teleport to="body">
            <Transition name="app-dropdown" :duration="{ enter: 300, leave: 200 }">
            <div v-if="thumbnailPopoverOpen" class="fixed inset-0 z-40" @click="thumbnailPopoverOpen = false" @wheel.prevent="thumbnailPopoverOpen = false">
              <div class="app-dropdown-surface app-frost-popover z-50 w-72 origin-top rounded-2xl p-3.5 shadow-xl"
                :style="thumbnailPopoverStyle"
                @click.stop @wheel.stop>
                <p class="mb-2 text-[11px] font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.thumbnailSize') }}</p>
                <div class="flex items-center gap-1.5" @wheel.prevent.stop="onThumbSizeWheel">
                  <IconZoomOut class="h-3.5 w-3.5 shrink-0" :class="isDark ? 'text-stone-400' : 'text-stone-500'" />
                  <AppSlider
                    v-model="thumbnailSize"
                    :min="0"
                    :max="thumbMaxIdx"
                    :tick="THUMB_DETAILS_MIN"
                    :tick-title="t('view.detailsMinTick')"
                    :is-dark="isDark"
                    :aria-label="t('albums.thumbnailSize')"
                    @wheel.prevent.stop="onThumbSizeWheel"
                  />
                  <IconZoomIn class="h-3.5 w-3.5 shrink-0" :class="isDark ? 'text-stone-400' : 'text-stone-500'" />
                  <button class="shrink-0 rounded p-0.5 transition"
                    :class="isDark ? 'text-stone-400 hover:text-stone-200 hover:bg-stone-700' : 'text-stone-400 hover:text-stone-600 hover:bg-stone-200'"
                    @click="thumbnailSize = THUMB_DEFAULT">
                    <IconRefresh class="h-3.5 w-3.5" />
                  </button>
                </div>
                <div class="mt-3 grid grid-cols-2 gap-1 rounded-xl p-0.5"
                  :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                  <button type="button"
                    class="relative inline-flex items-center justify-center gap-1 rounded-[0.65rem] px-2 py-1.5 text-[11px] font-medium transition"
                    :class="!viewLayout.compact
                      ? (isDark ? 'bg-stone-700 text-stone-50 shadow-sm' : 'bg-white text-stone-900 shadow-sm')
                      : (isDark ? 'text-stone-400 hover:text-stone-200' : 'text-stone-500 hover:text-stone-700')"
                    :title="forcedThumbsOnly ? t('view.detailsTooSmall') : undefined"
                    @click="setViewCompact(false)">
                    <IconList class="h-3.5 w-3.5" />
                    {{ t('view.details') }}
                    <IconAlertCircle
                      v-if="forcedThumbsOnly"
                      class="absolute -right-1 -top-1 h-3.5 w-3.5 text-amber-400 drop-shadow"
                    />
                  </button>
                  <button type="button"
                    class="inline-flex items-center justify-center gap-1 rounded-[0.65rem] px-2 py-1.5 text-[11px] font-medium transition"
                    :class="viewLayout.compact
                      ? (isDark ? 'bg-stone-700 text-stone-50 shadow-sm' : 'bg-white text-stone-900 shadow-sm')
                      : (isDark ? 'text-stone-400 hover:text-stone-200' : 'text-stone-500 hover:text-stone-700')"
                    @click="setViewCompact(true)">
                    <IconImage class="h-3.5 w-3.5" />
                    {{ t('view.thumbsOnly') }}
                  </button>
                </div>
                <button type="button"
                  class="mt-2 inline-flex w-full items-center justify-center gap-1.5 rounded-xl px-2.5 py-1.5 text-[11px] font-medium transition"
                  :class="isDark ? 'text-amber-200 hover:bg-stone-800' : 'text-amber-800 hover:bg-amber-50'"
                  @click="openViewSettings">
                  <IconGear class="h-3.5 w-3.5" />
                  {{ t('view.moreSettings') }}
                </button>
              </div>
            </div>
            </Transition>
          </teleport>
        </div>
        </div>
        </template>
      </div>
      <input ref="localFileInput" type="file" class="hidden" multiple accept="image/*,video/*,.raw,.arw,.nef,.cr2,.cr3,.orf,.rw2,.dng,.raf,.srw,.x3f,.3fr,.mef,.pef,.nrw,.rwl,.iiq,.erf,.fff" @change="handleLocalFileChange" />
      <ImportSourceModal
        :show="importSourceModalOpen"
        :is-dark="isDark"
        @local="handleImportLocal"
        @cloud="handleImportCloud"
        @close="importSourceModalOpen = false"
      />
      <CloudFilePicker
        :show="cloudPickerOpen"
        :is-dark="isDark"
        :fetch-list="fetchList"
        @confirm="handleCloudPickerConfirm"
        @close="cloudPickerOpen = false"
      />

      <!-- Selection action bar -->
      <div v-if="selectMode && !inSpecialView && !showingDesktopSearch && !embedded && !gradeView" class="flex items-center gap-2 app-split-b px-2.5 py-1.5 app-frost-toolbar md:px-4">
        <span class="text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
          {{ t('albums.selectedPhotos', { count: selectedPhotoPaths.size }) }}
        </span>
        <button class="rounded-lg px-2 py-1 text-xs font-medium transition"
          :class="isDark ? 'text-sky-300 hover:bg-stone-700' : 'text-sky-600 hover:bg-stone-200'"
          @click="selectAllPhotos">{{ t('albums.selectAll') }}</button>
        <button v-if="selectedPhotoPaths.size > 0" class="rounded-lg px-2 py-1 text-xs font-medium transition"
          :class="isDark ? 'text-stone-400 hover:bg-stone-700' : 'text-stone-500 hover:bg-stone-200'"
          @click="clearSelection">{{ t('albums.deselectAll') }}</button>
        <div class="flex-1"></div>
        <button v-if="selectedPhotoPaths.size > 0"
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition disabled:opacity-40"
          :class="isDark ? 'bg-emerald-700 text-white hover:bg-emerald-600' : 'bg-emerald-600 text-white hover:bg-emerald-700'"
          :disabled="exportSelectionPending"
          @click="openExportSelection">
          <IconDownload class="h-3.5 w-3.5" /><span class="hidden md:inline">{{ t('albums.exportSelected') }}</span>
        </button>
        <button v-if="selectedPhotoPaths.size > 0"
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition disabled:opacity-40"
          :class="isDark ? 'bg-stone-700 text-stone-100 hover:bg-stone-600' : 'bg-stone-800 text-white hover:bg-stone-700'"
          :disabled="downloadSelectionPending"
          @click="downloadSelected">
          <IconDownload class="h-3.5 w-3.5" /><span class="hidden md:inline">{{ t('albums.downloadSelected') }}</span>
        </button>
        <button v-if="selectedPhotoPaths.size > 0 && currentAlbumCanDelete"
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-rose-700 text-white hover:bg-rose-600' : 'bg-rose-600 text-white hover:bg-rose-700'"
          @click="confirmDeleteSelected">
          <IconTrash class="h-3.5 w-3.5" /><span class="hidden md:inline">{{ t('albums.deleteSelected') }}</span>
        </button>
      </div>
      </div>

      <!-- Main content: filter/sort sidebar + photo grid + AI tools -->
      <div class="flex min-h-0 flex-1 overflow-hidden">
        <!-- Desktop filter/sort sidebar -->
        <FilterSidebar
          v-if="!isMobile && filterSortPanelOpen && !gradeView"
          v-show="!inSpecialView && !showingDesktopSearch"
          :is-dark="isDark"
          :collections="collections"
          :selected-collection-id="selectedCollectionId"
          :available-tags="availableTags"
          :available-cameras="availableCameras"
          :available-lenses="availableLenses"
          :available-formats="availableFormats"
          :date-histogram="dateHistogram"
          :filters="filters"
          :sort="sort"
          :available-people="albumPeople"
          :current-user="currentUser"
          :current-album="currentAlbum"
          @apply="applyFilters"
          @update-sort="(s) => emit('update-sort', s)"
          @select-collection="(n) => emit('select-collection', n)"
          @close="filterSortPanelOpen = false"
        />

        <FilterSheet
          v-if="isMobile"
          :show="filterPanelOpen && !inSpecialView && !showingDesktopSearch"
          :is-dark="isDark"
          :collections="collections"
          :selected-collection-id="selectedCollectionId"
          :available-tags="availableTags"
          :available-cameras="availableCameras"
          :available-lenses="availableLenses"
          :available-formats="availableFormats"
          :date-histogram="dateHistogram"
          :filters="filters"
          :persist-filters="!isolateFilters"
          :available-people="albumPeople"
          :current-user="currentUser"
          :current-album="currentAlbum"
          @apply="applyFilters"
          @select-collection="(n) => emit('select-collection', n)"
          @close="filterPanelOpen = false"
        />
        <SortSheet
          v-if="isMobile"
          :show="sortPanelOpen && !inSpecialView && !showingDesktopSearch"
          :is-dark="isDark"
          :sort="sort"
          @update-sort="(s) => emit('update-sort', s)"
          @close="sortPanelOpen = false"
        />

        <GradeWorkspace
          v-if="gradeView && !isMobile"
          :photo="gradePhoto"
          :photos="gridPhotos"
          :is-dark="isDark"
          :format-size="formatSize"
          :available-tags="availableTags"
          :thumb-for="gradeThumb"
          @select="onGradeSelect"
          @download="downloadGradePhoto"
          @export="exportGradePhoto"
          @save-meta="saveGradeMeta"
          @load-more="handlePhotosScrollNearBottom"
        />

        <!-- Photo grid / duplicate groups -->
        <div
          ref="photoPaneRef"
          v-show="!gridPaused && !gradeView"
          class="album-no-drag relative min-h-0 flex-1 overflow-hidden"
          :class="!inSpecialView && !showingDesktopSearch && gridThumbsOnly ? 'p-0' : 'p-2 md:p-3'"
          :style="{ '--photo-gap': `${thumbGapPx}px`, touchAction: pinchLive ? 'none' : 'pan-y' }"
          @mousedown="preventAlbumDragLock"
          @dragstart.prevent
          @wheel.capture="onPhotoPaneWheel"
        >
          <div v-if="inDuplicatesView" class="absolute inset-0 overflow-hidden">
            <div v-if="busyAlbums && duplicateGroups.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loadingAlbum') }}</div>
            <div v-else-if="!busyAlbums && duplicateGroups.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.noDuplicateGroups') }}</div>
            <div v-else class="flex h-full min-h-0 flex-col">
              <div class="flex shrink-0 items-center justify-between px-1 pb-3">
                <p class="text-xs font-semibold uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.showDuplicates') }}</p>
                <p class="text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ duplicateTotal }}</p>
              </div>
              <div class="relative min-h-0 flex-1 overflow-hidden">
                <VList
                  :data="duplicateGroups"
                  class="app-scrollbar absolute inset-0"
                  :bufferSize="600"
                >
                  <template #default="{ item: group, index }">
                    <section class="pb-4 md:pb-5">
                      <div class="rounded-2xl app-surface app-card p-3 md:p-4">
                        <div class="mb-3 flex flex-wrap items-center gap-2">
                          <span class="rounded-full px-2.5 py-1 text-[11px] font-semibold"
                            :class="isDark ? 'bg-amber-900/40 text-amber-200' : 'bg-amber-100 text-amber-800'">
                            {{ group.kind === 'near' ? t('albums.duplicatesNear') : t('albums.duplicatesExact') }}
                          </span>
                          <span class="text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-700'">
                            {{ t('albums.duplicateGroupPhotos', { count: group.photo_count }) }}
                          </span>
                          <span v-if="group.kind === 'near'" class="truncate text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                            {{ t('albums.duplicatesDistance', { distance: group.max_hamming_distance }) }}
                          </span>
                          <span v-else-if="group.sha256" class="truncate text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                            {{ group.sha256.slice(0, 16) }}
                          </span>
                        </div>
                        <div class="grid" :class="thumbGridClass">
                          <article
                            v-for="photo in dupGroupPhotos(group, index)"
                            :key="photo.stable_id"
                            class="group min-w-0 self-start cursor-pointer rounded-xl app-surface app-card p-1.5 transition sm:p-2"
                            @click="openDuplicateDetail(photo)"
                          >
                            <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                              <LazyImage img-class="absolute inset-0 h-full w-full object-cover transition group-hover:scale-105" :src="getGridThumbnail(photo)" alt="" />
                            </div>
                            <p class="mt-1.5 truncate text-xs font-semibold" :title="duplicateDisplayName(photo)">{{ duplicateDisplayName(photo) }}</p>
                            <p class="truncate text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ formatSize(photo.size_bytes) }}</p>
                          </article>
                        </div>
                        <button
                          v-if="dupHiddenCount(group, index) > 0"
                          type="button"
                          class="mt-3 w-full rounded-xl px-3 py-2 text-xs font-medium transition"
                          :class="isDark ? 'bg-stone-800 text-stone-300 hover:bg-stone-700' : 'bg-stone-100 text-stone-600 hover:bg-stone-200'"
                          @click="expandDupGroup(group, index)"
                        >
                          {{ t('albums.duplicatesShowMore', { count: dupHiddenCount(group, index) }) }}
                        </button>
                      </div>
                    </section>
                  </template>
                </VList>
              </div>
            </div>
          </div>

          <div v-else-if="inTextView" class="app-scrollbar absolute inset-0 overflow-y-auto">
            <div class="mb-3 flex flex-wrap items-center gap-2 px-1">
              <button class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition"
                :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                @click="emit('show-photos')">
                <IconArrowLeft class="h-3.5 w-3.5" />{{ t('albums.backToPhotos') }}
              </button>
              <p class="text-xs font-semibold uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.textResults') }}</p>
              <p v-if="similarQuery?.text" class="truncate text-xs" :class="isDark ? 'text-stone-300' : 'text-stone-600'">"{{ similarQuery.text }}"</p>
              <p class="text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ similarQuery?.total ?? similarResults.length }}</p>
            </div>
            <div v-if="similarLoading" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loadingAlbum') }}</div>
            <div v-else-if="similarResults.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.noTextResults') }}</div>
            <div v-else class="grid" :class="thumbGridClass">
              <article v-for="photo in similarResults" :key="`${photo.owner}:${photo.path}`"
                class="group min-w-0 self-start cursor-pointer rounded-xl app-surface app-card p-1.5 transition sm:p-2"
                @click="openSimilarDetail(photo)">
                <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                  <LazyImage img-class="absolute inset-0 h-full w-full object-cover transition group-hover:scale-105" :src="getGridThumbnail(photo)" alt="" />
                </div>
                <p class="mt-1.5 truncate text-[11px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ photo.name }}</p>
                <p class="truncate text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  {{ t('albums.similarScore', { score: Number(photo.score || 0).toFixed(2) }) }}
                </p>
              </article>
            </div>
          </div>

          <div v-else-if="inSimilarView" class="app-scrollbar absolute inset-0 overflow-y-auto">
            <div class="mb-3 flex flex-wrap items-center gap-2 px-1">
              <button class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition"
                :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
                @click="emit('show-photos')">
                <IconArrowLeft class="h-3.5 w-3.5" />{{ t('albums.backToPhotos') }}
              </button>
              <p class="text-xs font-semibold uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.similarResults') }}</p>
              <p v-if="similarQuery?.name" class="truncate text-xs" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ similarQuery.name }}</p>
              <p class="text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ similarQuery?.total ?? similarResults.length }}</p>
            </div>
            <div v-if="similarLoading" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loadingAlbum') }}</div>
            <div v-else-if="similarResults.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.noSimilarResults') }}</div>
            <div v-else class="grid" :class="thumbGridClass">
              <article v-for="photo in similarResults" :key="`${photo.owner}:${photo.path}`"
                class="group min-w-0 self-start cursor-pointer rounded-xl app-surface app-card p-1.5 transition sm:p-2"
                @click="openSimilarDetail(photo)">
                <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                  <LazyImage img-class="absolute inset-0 h-full w-full object-cover transition group-hover:scale-105" :src="getGridThumbnail(photo)" alt="" />
                </div>
                <p class="mt-1.5 truncate text-[11px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ photo.name }}</p>
                <p class="truncate text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  {{ t('albums.similarScore', { score: Number(photo.score || 0).toFixed(2) }) }}
                </p>
              </article>
            </div>
          </div>

          <template v-else>
          <div v-if="desktopSearchLoading || (!showingDesktopSearch && (albumPhotoLoadHold || (busyAlbums && albumPhotos.length === 0)))" class="flex h-full flex-col items-center justify-center gap-3">
            <div class="h-8 w-8 animate-spin rounded-full border-2 border-amber-500 border-t-transparent"></div>
            <p class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loadingAlbum') }}</p>
          </div>
          <div v-else-if="gridPhotos.length === 0 && (showingDesktopSearch || !busyAlbums)" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ showingDesktopSearch ? (desktopSearchError || t('albums.noTextResults')) : (emptyLabel || t('albums.noPhotosFiltered')) }}</div>

          <VirtualGrid
            v-else
            ref="photoGridRef"
            :items="gridPhotos"
            item-key="path"
            :grid-class="'grid ' + thumbGridClass"
            :zoom-level="thumbnailSize"
            :zoom-classes="thumbZoomClasses"
            :zoom-gaps="thumbZoomGaps"
            :image-src="getGridThumbnail"
            :paused="gridPaused"
            class="absolute inset-0"
            @scroll-near-bottom="handlePhotosScrollNearBottom"
            @zooming="onGridZooming"
          >
            <template #default="{ item: photo }">
            <article
              v-if="showingDesktopSearch"
              class="group min-w-0 cursor-pointer self-start rounded-xl app-surface app-card p-1.5 transition sm:p-2"
              @click="onGridPhotoClick(photo)"
            >
              <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                <LazyImage img-class="absolute inset-0 h-full w-full object-cover transition group-hover:scale-105" :src="getGridThumbnail(photo)" alt="" />
              </div>
              <p class="mt-1.5 truncate text-[11px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ photo.name }}</p>
              <p class="truncate text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                {{ t('albums.similarScore', { score: Number(photo.score || 0).toFixed(2) }) }}
              </p>
            </article>
            <PhotoCard
              v-else
              :photo="photo"
              :layout="viewLayout"
              :compact="gridCompact"
              :hide-overlays="gridHideOverlays"
              :is-dark="isDark"
              :thumb-src="getGridThumbnail(photo)"
              :format-size="formatSize"
              :select-mode="selectMode"
              :selected="selectMode && selectedPhotoPaths.has(photo.path)"
              @pointerdown="onPhotoPointerDown(photo, $event)"
              @contextmenu="onPhotoContextMenu(photo, $event)"
              @click="onGridPhotoClick(photo)"
            />
            </template>
          </VirtualGrid>
          </template>
        </div>

        <GradeResources v-if="gradeView && resourcesOpen && !isSharedAlbum" :is-dark="isDark" @close="resourcesOpen = false" />

        <!-- AI tools modal (desktop) -->
        <teleport v-if="!embedded" to="body">
          <AlbumAiToolsModal
            :show="aiToolsPanelOpen"
            :is-dark="isDark"
            :is-mobile="isMobile"
            :initial-section="aiToolsInitialSection"
            :album-name="currentAlbum"
            :album-owner="albumFacesOwner || currentUser"
            :busy="albumPeopleLoading"
            :hints-loading="albumFaceHintsLoading"
            :people="albumPeople"
            :suggestions="albumFaceSuggestions"
            :editable="albumFacesEditable"
            :cluster-similarity="albumClusterSimilarity"
            :reclustering="albumClusterReclustering"
            :initial-query="textSearchInput"
            :on-search-text="onSearchTextInModal"
            :on-load-duplicates="onLoadDuplicatesInModal"
            @close="aiToolsPanelOpen = false"
            @back="aiToolsPanelOpen = false"
            @refresh="emit('refresh-album-people')"
            @adopt-cluster="(body) => emit('adopt-album-cluster', body)"
            @recluster="(similarity) => emit('recluster-album-faces', similarity)"
            @merge-cluster="(body) => emit('merge-album-cluster', body)"
            @patch-person="(id, updates) => emit('patch-album-person', id, updates)"
            @remove-center="(body) => emit('remove-album-center', body)"
            @delete-person="(id) => emit('delete-album-person', id)"
            @open-photo="onAiToolsOpenPhoto"
          />
        </teleport>
      </div>
    </div>
    </component>

    <!-- ═══════════════════════════════════════════
         Album Create / Edit Dialog
         ═══════════════════════════════════════════ -->
    <AppDialog :show="showAlbumDialog" :z-index="100" @close="showAlbumDialog = false">
        <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-md rounded-2xl border p-5"
          :class="isDark ? 'text-stone-100' : 'text-stone-900'">
          <h3 class="mb-4 text-base font-bold">{{ albumDialogMode === 'create' ? t('albums.createAlbum') : t('albums.editAlbum') }}</h3>
          <div class="flex flex-col gap-3">
            <div>
              <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('albums.albumName') }}</label>
              <input v-model="albumDialogForm.name" type="text" :placeholder="t('albums.albumNamePlaceholder')"
                class="w-full rounded-lg border px-3 py-2 text-sm"
                :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
                @keyup.enter="submitAlbumDialog" />
            </div>
            <div>
              <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('albums.notes') }}</label>
              <textarea v-model="albumDialogForm.description" rows="3" :placeholder="t('albums.notesPlaceholder')"
                class="w-full rounded-lg border px-3 py-2 text-sm"
                :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
            </div>
            <div v-if="albumDialogMode === 'edit'">
              <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('albums.coverPath') }}</label>
              <input v-model="albumDialogForm.cover" type="text" :placeholder="t('albums.coverPlaceholder')"
                class="w-full rounded-lg border px-3 py-2 text-sm"
                :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
            </div>
          </div>
          <div class="mt-4 flex justify-end gap-2">
            <button class="rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
              @click="showAlbumDialog = false">{{ t('albums.cancelBtn') }}</button>
            <button class="rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
              @click="submitAlbumDialog">{{ albumDialogMode === 'create' ? t('albums.createBtn') : t('albums.saveBtn') }}</button>
          </div>
        </div>
    </AppDialog>

      <ViewSettingsDialog
        v-if="!embedded"
        :show="viewSettingsOpen"
        v-model="viewLayout"
        :is-dark="isDark"
        :format-size="formatSize"
        @close="viewSettingsOpen = false"
      />

    <!-- ═══════════════════════════════════════════
         Share Dialog
         ═══════════════════════════════════════════ -->
    <AppDialog :show="showShareDialog" :z-index="100" @close="showShareDialog = false">
        <div
          class="app-dialog-panel app-dialog-panel-dense share-dialog flex w-full flex-col rounded-2xl border p-5"
          :class="[isDark ? 'text-stone-100' : 'text-stone-900', isMobile ? 'max-w-md' : 'max-w-lg']"
        >
          <header class="mb-4 shrink-0">
            <h3 class="text-base font-bold">{{ t('share.title') }}</h3>
            <p class="mt-1 truncate text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
              {{ shareDialogAlbum?.name || shareDialogAlbum?.dir_name }}
            </p>
          </header>

          <!-- Mobile: stacked cards -->
          <template v-if="isMobile">
            <section class="mb-5 shrink-0">
              <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wide"
                :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('share.addUser') }}</h4>
              <input
                v-model="shareForm.username"
                type="text"
                :placeholder="t('share.usernamePlaceholder')"
                class="w-full rounded-xl border px-3.5 py-2.5 text-sm"
                :class="isDark ? 'border-stone-700 bg-stone-800/80 text-stone-100 placeholder:text-stone-500' : 'border-stone-200 bg-white text-stone-900 placeholder:text-stone-400'"
                :disabled="shareMutationPending"
                @keyup.enter="submitShare"
              />
              <div class="mt-2 overflow-hidden rounded-xl border"
                :class="isDark ? 'border-stone-700/80' : 'border-stone-200'">
                <label class="share-perm-row app-switch">
                  <span class="app-switch-label">{{ t('share.canEdit') }}</span>
                  <input
                    type="checkbox"
                    :checked="shareForm.canWrite"
                    :disabled="shareMutationPending"
                    @change="setShareFormWrite($event.target.checked)"
                  />
                  <span class="app-switch-track"></span>
                </label>
                <div class="mx-3.5" :class="isDark ? 'border-t border-stone-700/80' : 'border-t border-stone-200'"></div>
                <label class="share-perm-row app-switch">
                  <span class="app-switch-label">{{ t('share.canDelete') }}</span>
                  <input
                    type="checkbox"
                    :checked="shareForm.canDelete"
                    :disabled="shareMutationPending"
                    @change="setShareFormDelete($event.target.checked)"
                  />
                  <span class="app-switch-track"></span>
                </label>
              </div>
              <button
                type="button"
                class="app-btn app-btn-primary mt-3 w-full py-2.5 text-sm"
                :disabled="shareMutationPending || !shareForm.username.trim()"
                @click="submitShare"
              >
                <IconPlus class="h-4 w-4" />
                {{ t('share.add') }}
              </button>
            </section>

            <section class="min-h-0 flex-1 overflow-y-auto">
              <h4 class="mb-2 text-[11px] font-semibold uppercase tracking-wide"
                :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ t('share.people') }}</h4>
              <div v-if="shareDialogAlbum?.shares?.length" class="flex flex-col gap-2">
                <article
                  v-for="s in shareDialogAlbum.shares"
                  :key="s.username"
                  class="overflow-hidden rounded-xl border"
                  :class="isDark ? 'border-stone-700/80' : 'border-stone-200'"
                >
                  <div class="flex items-center gap-2 px-3.5 pt-3 pb-1">
                    <span class="min-w-0 flex-1 truncate text-sm font-semibold">{{ s.username }}</span>
                    <button
                      type="button"
                      class="flex h-8 w-8 items-center justify-center rounded-lg transition"
                      :class="isDark ? 'text-rose-400 hover:bg-rose-900/30' : 'text-rose-500 hover:bg-rose-100'"
                      :title="t('share.remove')"
                      :aria-label="t('share.remove')"
                      :disabled="shareMutationPending"
                      @click="removeShare(shareDialogAlbum, s.username)"
                    >
                      <IconTrash class="h-4 w-4" />
                    </button>
                  </div>
                  <label class="share-perm-row app-switch">
                    <span class="app-switch-label">{{ t('share.canEdit') }}</span>
                    <input
                      type="checkbox"
                      :checked="s.can_write"
                      :disabled="shareMutationPending"
                      @change="updateSharePermission(s, 'can_write', $event.target.checked)"
                    />
                    <span class="app-switch-track"></span>
                  </label>
                  <div class="mx-3.5" :class="isDark ? 'border-t border-stone-700/80' : 'border-t border-stone-200'"></div>
                  <label class="share-perm-row app-switch">
                    <span class="app-switch-label">{{ t('share.canDelete') }}</span>
                    <input
                      type="checkbox"
                      :checked="s.can_delete"
                      :disabled="shareMutationPending"
                      @change="updateSharePermission(s, 'can_delete', $event.target.checked)"
                    />
                    <span class="app-switch-track"></span>
                  </label>
                </article>
              </div>
              <div
                v-else
                class="rounded-xl px-3 py-6 text-center text-sm"
                :class="isDark ? 'bg-stone-800/70 text-stone-400' : 'bg-stone-50 text-stone-500'"
              >
                {{ t('share.noShares') }}
              </div>
            </section>
          </template>

          <!-- Desktop: one person per table row -->
          <template v-else>
            <div
              v-if="shareDialogAlbum?.shares?.length"
              class="mb-4 min-h-0 flex-1 overflow-hidden rounded-xl border"
              :class="isDark ? 'border-stone-700/80' : 'border-stone-200'"
            >
              <div class="max-h-[min(22rem,48vh)] overflow-y-auto">
                <table class="w-full text-sm">
                  <thead class="sticky top-0">
                    <tr :class="isDark ? 'bg-stone-800 text-stone-400' : 'bg-stone-50 text-stone-500'">
                      <th class="px-3 py-2 text-left text-xs font-medium">{{ t('share.user') }}</th>
                      <th class="px-3 py-2 text-center text-xs font-medium">{{ t('share.canEdit') }}</th>
                      <th class="px-3 py-2 text-center text-xs font-medium">{{ t('share.canDelete') }}</th>
                      <th class="w-10 px-3 py-2"></th>
                    </tr>
                  </thead>
                  <tbody>
                    <tr
                      v-for="s in shareDialogAlbum.shares"
                      :key="s.username"
                      class="border-t"
                      :class="isDark ? 'border-stone-700' : 'border-stone-200'"
                    >
                      <td class="px-3 py-2.5 font-medium">{{ s.username }}</td>
                      <td class="px-3 py-2.5">
                        <div class="flex justify-center">
                          <label class="app-switch">
                            <input
                              type="checkbox"
                              :checked="s.can_write"
                              :disabled="shareMutationPending"
                              @change="updateSharePermission(s, 'can_write', $event.target.checked)"
                            />
                            <span class="app-switch-track"></span>
                          </label>
                        </div>
                      </td>
                      <td class="px-3 py-2.5">
                        <div class="flex justify-center">
                          <label class="app-switch">
                            <input
                              type="checkbox"
                              :checked="s.can_delete"
                              :disabled="shareMutationPending"
                              @change="updateSharePermission(s, 'can_delete', $event.target.checked)"
                            />
                            <span class="app-switch-track"></span>
                          </label>
                        </div>
                      </td>
                      <td class="px-3 py-2.5 text-center">
                        <button
                          type="button"
                          class="rounded p-1 transition"
                          :class="isDark ? 'text-rose-400 hover:bg-rose-900/30' : 'text-rose-500 hover:bg-rose-100'"
                          :title="t('share.remove')"
                          :aria-label="t('share.remove')"
                          :disabled="shareMutationPending"
                          @click="removeShare(shareDialogAlbum, s.username)"
                        >
                          <IconTrash class="h-3.5 w-3.5" />
                        </button>
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>
            <div
              v-else
              class="mb-4 rounded-xl px-3 py-6 text-center text-sm"
              :class="isDark ? 'bg-stone-800/70 text-stone-400' : 'bg-stone-50 text-stone-500'"
            >
              {{ t('share.noShares') }}
            </div>

            <div class="flex items-end gap-3">
              <div class="min-w-0 flex-1">
                <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
                  {{ t('share.addUser') }}
                </label>
                <input
                  v-model="shareForm.username"
                  type="text"
                  :placeholder="t('share.usernamePlaceholder')"
                  class="w-full rounded-xl border px-3 py-2 text-sm"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-200 bg-white text-stone-900 placeholder:text-stone-400'"
                  :disabled="shareMutationPending"
                  @keyup.enter="submitShare"
                />
              </div>
              <label class="app-switch shrink-0">
                <span class="app-switch-label">{{ t('share.canEdit') }}</span>
                <input
                  type="checkbox"
                  :checked="shareForm.canWrite"
                  :disabled="shareMutationPending"
                  @change="setShareFormWrite($event.target.checked)"
                />
                <span class="app-switch-track"></span>
              </label>
              <label class="app-switch shrink-0">
                <span class="app-switch-label">{{ t('share.canDelete') }}</span>
                <input
                  type="checkbox"
                  :checked="shareForm.canDelete"
                  :disabled="shareMutationPending"
                  @change="setShareFormDelete($event.target.checked)"
                />
                <span class="app-switch-track"></span>
              </label>
              <button
                type="button"
                class="app-btn app-btn-primary shrink-0 px-3 py-2 text-sm"
                :disabled="shareMutationPending || !shareForm.username.trim()"
                @click="submitShare"
              >
                <IconPlus class="h-4 w-4" />
                {{ t('share.add') }}
              </button>
            </div>
          </template>

          <div class="mt-4 flex shrink-0 justify-end">
            <button
              type="button"
              class="app-btn app-btn-secondary px-4 py-2 text-sm"
              @click="showShareDialog = false"
            >{{ t('share.close') }}</button>
          </div>
        </div>
    </AppDialog>

    <!-- ═══════════════════════════════════════════
         Photo Detail / Edit overlay
         ═══════════════════════════════════════════ -->
    <AppDialog :show="!embedded && !!detailPhoto && !detailClosing" :z-index="130" :center-class="detailFullscreen ? 'p-0' : 'p-0 md:p-4'" @close="closeDetail" @after-leave="finishCloseDetail">
        <div ref="detailPanelRef" tabindex="-1" class="app-dialog-panel flex overflow-hidden outline-none"
          :class="[
            detailFullscreen
              ? 'app-dialog-solid app-photo-fullscreen text-white'
              : [
                  'relative border',
                  'app-dialog-panel-soft',
                  isDark ? 'text-stone-100' : 'text-stone-900',
                  isMobile ? 'h-full w-full flex-col' : 'w-full flex-col rounded-2xl',
                  detailExpanded && !isMobile ? 'md:mx-auto md:h-[80vh] md:w-[80vw] md:rounded-2xl' : 'md:mx-4 md:max-h-[90vh] md:max-w-5xl md:rounded-2xl'
                ],
          ]">

          <!-- Top bar with nav + actions -->
          <div v-if="!detailFullscreen" class="flex items-center gap-1.5 border-b px-3 py-2"
            :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'">
            <button class="rounded-full p-1.5 transition"
              :class="hasPrev ? (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300') : 'opacity-30 cursor-default'"
              :disabled="!hasPrev" @click="goDetailPrev">
              <IconChevronLeft class="h-4 w-4" />
            </button>
            <button class="rounded-full p-1.5 transition"
              :class="hasNext ? (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300') : 'opacity-30 cursor-default'"
              :disabled="!hasNext" @click="goDetailNext">
              <IconChevronRight class="h-4 w-4" />
            </button>
            <span class="ml-1 text-xs tabular-nums" :class="isDark ? 'text-stone-400' : 'text-stone-500'"
              v-if="detailIndex >= 0">{{ detailIndex + 1 }} / {{ detailPhotoList.length }}</span>
            <div class="flex-1"></div>
            <button class="rounded-full p-1.5 transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              :title="t('detail.fullscreen')"
              @click="toggleDetailFullscreen">
              <IconExpand class="h-4 w-4" />
            </button>
            <button v-if="!isDetailVideo" class="inline-flex items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium transition"
              :class="detailExpanded
                ? (isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900')
                : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')"
              @click="toggleExpanded">
              <IconViewOriginal class="h-3.5 w-3.5" />
              <span class="hidden md:inline">{{ detailExpanded ? t('detail.collapse') : t('detail.viewOriginal') }}</span>
            </button>
            <button v-if="!isDetailVideo" class="inline-flex items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium transition"
              :class="darktablePreview
                ? (isDark ? 'bg-emerald-500/20 text-emerald-200' : 'bg-emerald-100 text-emerald-900')
                : hasXmpSidecar
                  ? (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')
                  : 'opacity-30 cursor-default'"
              :disabled="!hasXmpSidecar"
              :title="!hasXmpSidecar ? t('detail.noXmp') : t('detail.darktablePreview')"
              @click="toggleDarktablePreview">
              <IconCamera class="h-3.5 w-3.5" />
              <span class="hidden md:inline">{{ t('detail.darktablePreview') }}</span>
            </button>
            <button class="rounded-full p-1.5 transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              @click="closeDetail">
              <IconClose class="h-4 w-4" />
            </button>
          </div>

          <div class="flex min-h-0 flex-1 flex-col p-4 md:flex-row md:gap-5 md:overflow-hidden md:p-5"
            :class="detailFullscreen ? 'app-photo-fullscreen-body !overflow-hidden !p-0' : (isMobile ? 'overflow-hidden' : 'overflow-y-auto')">
            <!-- Preview -->
            <div class="group relative flex min-h-0 min-w-0 flex-1 items-center justify-center"
              :class="detailFullscreen ? 'app-photo-fullscreen-stage' : ''"
              @mousemove="showFullscreenControls"
              @wheel.prevent="onDetailWheel">
              <!-- Loading spinner -->
              <div v-if="imageLoading" class="absolute inset-0 z-10 flex items-center justify-center">
                <svg class="h-8 w-8 animate-spin" :class="isDark ? 'text-stone-400' : 'text-stone-500'" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
                </svg>
              </div>
              <video
                v-if="isDetailVideo && detailVideoLive && detailVideoCanPlay && !videoUnplayable"
                ref="detailVideoRef"
                :key="detailPlaybackSrc"
                :src="detailPlaybackSrc"
                class="rounded-xl bg-black object-contain"
                :style="{ background: `#000 url('${detailImageSrc}') center / contain no-repeat` }"
                :class="detailFullscreen
                  ? 'app-photo-fullscreen-img'
                  : isMobile
                    ? 'absolute inset-0 h-full w-full'
                    : (detailExpanded ? 'max-h-64 w-full sm:max-h-[60vh] md:max-h-full' : 'max-h-56 w-full sm:max-h-96 md:max-h-[72vh]')"
                controls
                playsinline
                @loadeddata="onDetailVideoReady"
                @error="onDetailVideoError"
              />
              <img v-else :key="detailImageEpoch"
                :src="detailImageSrc"
                class="rounded-xl object-contain transition-opacity"
                :class="[
                  detailFullscreen
                    ? 'app-photo-fullscreen-img'
                    : isMobile
                      ? 'absolute inset-0 h-full w-full'
                      : (detailExpanded ? 'max-h-64 sm:max-h-[60vh] md:max-h-full' : 'max-h-56 sm:max-h-96 md:max-h-[72vh]'),
                  imageLoading ? 'opacity-40' : 'opacity-100'
                ]"
                alt="" @load="onDetailImageLoad" @error="onDetailImageError" />
              <div
                v-if="isDetailVideo && detailVideoPlan?.badge"
                class="pointer-events-none absolute right-3 top-3 z-30 max-w-[70%] truncate rounded-md bg-black/70 px-2 py-1 text-[11px] font-medium text-white opacity-100 [@media(hover:hover)]:opacity-0 [@media(hover:hover)]:group-hover:opacity-100"
              >{{ detailVideoPlan.badge }}</div>
              <div
                v-if="isDetailVideo && detailVideoPlan?.mode === 'ask'"
                class="absolute inset-0 z-20 flex items-center justify-center p-6"
              >
                <div class="flex max-w-sm flex-col items-center gap-3 rounded-xl bg-black/75 px-4 py-4 text-center text-sm text-white">
                  <p>{{ detailVideoPlan.ask }}</p>
                  <button
                    type="button"
                    class="rounded-full bg-white/15 px-3 py-1.5 text-xs font-medium hover:bg-white/25 disabled:opacity-60"
                    :disabled="detailVideoEncoding"
                    @click="confirmDetailVideoEncode"
                  >{{ detailVideoEncoding ? t('detail.videoTranscoding') : t('detail.videoTranscode') }}</button>
                </div>
              </div>
              <div
                v-if="isDetailVideo && videoUnplayable"
                class="absolute inset-x-6 bottom-6 z-20 flex flex-col items-center gap-2 rounded-xl bg-black/70 px-4 py-3 text-center text-sm text-white"
              >
                <p>{{ t('detail.videoUnplayable') }}</p>
                <button
                  type="button"
                  class="rounded-full bg-white/15 px-3 py-1.5 text-xs font-medium hover:bg-white/25"
                  @click="downloadOriginal"
                >{{ t('detail.download') }}</button>
              </div>

              <div v-if="detailFullscreen"
                class="pointer-events-none absolute left-4 top-4 max-w-[calc(100%-10rem)] rounded-md bg-black/40 px-3 py-2 text-white backdrop-blur-sm transition-opacity"
                :class="detailFullscreenControlsVisible ? 'opacity-100' : 'opacity-0'">
                <p v-if="detailIndex >= 0" class="text-xs tabular-nums text-white/80">{{ detailIndex + 1 }} / {{ detailPhotoList.length }}</p>
                <p class="truncate text-sm font-medium" :title="detailPhoto.name">{{ detailPhoto.name }}</p>
              </div>

              <button v-if="detailFullscreen"
                class="absolute left-5 top-1/2 -translate-y-1/2 rounded-full bg-black/50 p-3 text-white transition hover:bg-black/75"
                :class="[detailFullscreenControlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0', hasPrev ? '' : 'pointer-events-none']"
                :disabled="!hasPrev"
                :title="t('detail.previous')"
                @click="showFullscreenControls(); goDetailPrev()">
                <IconChevronLeft class="h-6 w-6" />
              </button>
              <button v-if="detailFullscreen"
                class="absolute right-5 top-1/2 -translate-y-1/2 rounded-full bg-black/50 p-3 text-white transition hover:bg-black/75"
                :class="[detailFullscreenControlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0', hasNext ? '' : 'pointer-events-none']"
                :disabled="!hasNext"
                :title="t('detail.next')"
                @click="showFullscreenControls(); goDetailNext()">
                <IconChevronRight class="h-6 w-6" />
              </button>
              <div v-if="detailFullscreen" class="absolute right-4 top-4 flex gap-2 transition-opacity"
                :class="detailFullscreenControlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0'">
                <button
                  class="rounded-full bg-black/50 p-2.5 text-white transition hover:bg-black/75"
                  :title="t('detail.showDetails')"
                  @click="toggleDetailFullscreen">
                  <IconCollapse class="h-5 w-5" />
                </button>
                <button
                  class="rounded-full bg-black/50 p-2.5 text-white transition hover:bg-black/75"
                  :title="t('detail.close')"
                  @click="closeDetail">
                  <IconClose class="h-5 w-5" />
                </button>
              </div>
            </div>

            <!-- Meta & editor -->
            <div v-if="!detailFullscreen" class="app-scrollbar mt-3 flex w-full shrink-0 flex-col gap-2.5 overflow-y-auto md:mt-0 md:w-72 md:max-h-none md:gap-3 lg:w-80"
              :class="isMobile ? 'max-h-[42%]' : ''">
              <PhotoMetaPanel
                :photo="detailPhoto"
                :is-dark="isDark"
                :format-size="formatSize"
                :dimensions="detailDimensions || ''"
                :duration-label="detailDurationLabel || ''"
                :rating="editRating"
                :color-label-keys="editColorLabels"
                :tags="editTags"
                :available-tags="availableTags"
                :can-write="currentAlbumCanWrite"
                :can-delete="currentAlbumCanDelete"
                :is-video="isDetailVideo"
                :show-tune="!isMobile && !isDetailVideo"
                @download="downloadOriginal"
                @export="openExportDialog"
                @tune="toggleGradeView"
                @set-rating="setDetailRating"
                @toggle-color="setDetailColorLabel"
                @update:tags="editTags = $event"
                @set-cover="setCoverFromPhoto(detailPhoto)"
                @add-to-album="openAddToAlbumModal(detailPhoto)"
                @find-similar="findSimilarFromDetail"
                @delete="confirmDeleteSinglePhoto"
                @save="savePhotoMeta"
              />
            </div>
          </div>
        </div>
    </AppDialog>

    <!-- ═══════════════════════════════════════════
         Export Dialog
         ═══════════════════════════════════════════ -->
    <ActionSheet
      :show="photoMenuOpen"
      :title="menuPhoto?.name"
      :actions="photoMenuActions"
      @close="photoMenuOpen = false"
      @select="onPhotoMenuAction"
    />
    <ActionSheet
      :show="photoAlbumOpen"
      :title="t('detail.addToAlbum')"
      :actions="photoMenuAlbumActions"
      @close="closePhotoAlbumMenu"
      @select="onPhotoMenuAlbum"
    />

    <AppDialog :show="showExportDialog" :z-index="145" @close="showExportDialog = false; exportSelection = false; menuPhoto = null">
        <div class="app-dialog-panel app-dialog-panel-dense w-full max-w-sm rounded-2xl border p-5"
          :class="isDark ? 'text-stone-100' : 'text-stone-900'">
          <h3 class="mb-4 text-base font-bold">{{ exportSelection ? t('albums.exportSelected') : t('detail.exportTitle') }}</h3>

          <!-- Format -->
          <div class="mb-3">
            <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.exportFormat') }}</label>
            <select v-model="exportForm.format"
              class="w-full rounded-lg border px-2.5 py-1.5 text-sm"
              :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100' : 'border-stone-300 bg-white text-stone-900'">
              <option value="jpeg">JPEG</option>
              <option value="png">PNG</option>
              <option value="tiff">TIFF</option>
              <option value="webp">WebP</option>
            </select>
          </div>

          <!-- Quality -->
          <div class="mb-3" v-if="exportForm.format === 'jpeg' || exportForm.format === 'webp'">
            <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.exportQuality') }}: {{ exportForm.quality }}</label>
            <AppSlider v-model="exportForm.quality" :min="1" :max="100" :is-dark="isDark" />
          </div>

          <!-- Max width -->
          <div class="mb-4">
            <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.exportMaxWidth') }}</label>
            <input type="number" v-model.number="exportForm.maxWidth" min="0" step="100"
              :placeholder="t('detail.exportMaxWidthPlaceholder')"
              class="w-full rounded-lg border px-2.5 py-1.5 text-sm"
              :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
          </div>

          <!-- Grading -->
          <div class="mb-4 space-y-3">
            <div class="flex items-center justify-between gap-3">
              <label class="text-xs font-medium" :class="exportHasXmp ? (isDark ? 'text-stone-300' : 'text-stone-600') : (isDark ? 'text-stone-600' : 'text-stone-400')">
                {{ t('detail.exportApplyXmp') }}
                <span v-if="!exportHasXmp" class="ml-1 font-normal opacity-70">{{ t('detail.noXmp') }}</span>
              </label>
              <label class="app-switch">
                <input type="checkbox" v-model="exportForm.applyXmp" :disabled="!exportHasXmp" @change="onToggleApplyXmp" />
                <span class="app-switch-track"></span>
              </label>
            </div>
            <div class="flex items-center justify-between gap-3">
              <label class="text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
                {{ t('detail.exportApplyGrade') }}
              </label>
              <label class="app-switch">
                <input type="checkbox" v-model="exportForm.applyGrade" @change="onToggleApplyGrade" />
                <span class="app-switch-track"></span>
              </label>
            </div>
          </div>
          <p v-if="exportDirectCopy" class="mb-3 text-[11px] leading-snug" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
            {{ t('detail.exportDirectHint') }}
          </p>

          <!-- Actions -->
          <div class="flex gap-2">
            <button class="flex-1 rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
              @click="showExportDialog = false; exportSelection = false">{{ t('share.close') }}</button>
            <button class="flex-1 rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-emerald-600 text-white hover:bg-emerald-500' : 'bg-emerald-600 text-white hover:bg-emerald-700'"
              :disabled="exporting"
              @click="doExport">
              {{ exporting ? t('detail.exporting') : t('detail.exportBtn') }}
            </button>
          </div>
        </div>
    </AppDialog>

    <!-- ═══════════════════════════════════════════
         Add to Album Modal
         ═══════════════════════════════════════════ -->
    <AppDialog :show="!embedded && showAddToAlbumModal" :z-index="145" @close="showAddToAlbumModal = false">
        <div class="app-dialog-panel app-dialog-panel-dense flex max-h-[80vh] w-full max-w-md flex-col rounded-2xl border"
          :class="isDark ? 'text-stone-100' : 'text-stone-900'">
          <div class="flex items-center justify-between border-b px-5 py-4"
            :class="isDark ? 'border-stone-700' : 'border-stone-200'">
            <h3 class="text-base font-bold">{{ t('detail.addToAlbum') }}</h3>
            <button class="rounded-full p-1.5 transition"
              :class="isDark ? 'text-stone-400 hover:bg-stone-800 hover:text-stone-200' : 'text-stone-500 hover:bg-stone-100'"
              @click="showAddToAlbumModal = false">
              <IconClose class="h-4 w-4" />
            </button>
          </div>
          <div class="app-scrollbar flex flex-col overflow-y-auto p-3">
            <p v-if="addableAlbums.length === 0"
              class="py-8 text-center text-sm"
              :class="isDark ? 'text-stone-500' : 'text-stone-400'">
              {{ t('detail.noOtherAlbums') }}
            </p>
            <button
              v-for="album in addableAlbums"
              :key="album.dir_name"
              class="flex items-center gap-3 rounded-xl px-3 py-2.5 text-left transition"
              :class="isDark ? 'hover:bg-stone-800' : 'hover:bg-stone-100'"
              :disabled="addToAlbumLoading"
              @click="doAddToAlbum(album)">
              <!-- Cover thumbnail -->
              <div class="h-12 w-12 shrink-0 overflow-hidden rounded-lg"
                :class="isDark ? 'bg-stone-700' : 'bg-stone-200'">
                <img v-if="album.cover"
                  :src="getAddableAlbumCover(album)"
                  class="h-full w-full object-cover"
                  loading="lazy" alt="" />
                <div v-else class="flex h-full w-full items-center justify-center">
                  <IconImage class="h-5 w-5" :class="isDark ? 'text-stone-500' : 'text-stone-400'" />
                </div>
              </div>
              <!-- Name + owner -->
              <div class="min-w-0 flex-1">
                <p class="truncate text-sm font-medium">{{ album.name || album.dir_name }}</p>
                <p v-if="album.owner" class="truncate text-xs"
                  :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                  {{ album.owner }}
                </p>
              </div>
            </button>
          </div>
        </div>
    </AppDialog>

    <teleport to="body">
      <div class="pointer-events-none fixed inset-x-0 z-[80] flex justify-center" :style="{ bottom: 'calc(var(--safe-bottom, 0px) + 2rem)' }">
        <Transition name="thumb-size-toast">
          <div
            v-if="thumbSizeToastOpen"
            class="pointer-events-auto flex items-center gap-2.5 rounded-full border px-3.5 py-2 shadow-lg backdrop-blur-xl"
            :class="isDark
              ? 'border-white/10 bg-stone-950/55 text-stone-200'
              : 'border-white/50 bg-white/45 text-stone-700'"
          >
            <IconZoomOut class="h-3.5 w-3.5 shrink-0 opacity-50" />
            <div class="w-44 shrink-0">
              <AppSlider
                v-model="thumbnailSize"
                :min="0"
                :max="thumbMaxIdx"
                :tick="THUMB_DETAILS_MIN"
                :tick-title="t('view.detailsMinTick')"
                tone="ghost"
                :aria-label="t('albums.thumbnailSize')"
                @input="showThumbSizeToast"
                @wheel.prevent.stop="onThumbToastWheel"
              />
            </div>
            <IconZoomIn class="h-3.5 w-3.5 shrink-0 opacity-50" />
          </div>
        </Transition>
      </div>
    </teleport>
  </section>
</template>

<style scoped>
.thumb-size-toast-enter-active,
.thumb-size-toast-leave-active {
  transition: opacity 180ms ease, transform 180ms ease;
}
.thumb-size-toast-enter-from,
.thumb-size-toast-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
.share-dialog {
  max-height: min(88vh, 40rem);
  overflow: hidden;
}

.share-perm-row {
  width: 100%;
  min-height: 2.75rem;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.15rem 0.9rem;
}

.share-perm-row .app-switch-label {
  font-size: 0.875rem;
  font-weight: 500;
}

.share-perm-row :deep(.app-switch-track) {
  width: 2.2rem;
  height: 1.28rem;
}

.share-perm-row :deep(.app-switch-track::after) {
  width: 0.98rem;
  height: 0.98rem;
}

.share-perm-row :deep(input:checked + .app-switch-track::after) {
  transform: translateX(0.92rem);
}

@media (max-width: 767px) {
  .album-view-enter-active,
  .album-view-leave-active {
    transition: opacity 180ms ease, transform 180ms ease;
  }

  .album-view-enter-from {
    opacity: 0;
    transform: translateX(16px);
  }

  .album-view-leave-to {
    opacity: 0;
    transform: translateX(-10px);
  }
}
</style>
