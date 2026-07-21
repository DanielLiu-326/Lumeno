<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch } from "vue";
import {
  IconFolderAdd, IconRefresh, IconStar, IconStarOff, IconFilter, IconDatabase,
  IconCamera, IconClose, IconChevronLeft, IconChevronRight, IconExpand,
  IconArrowLeft, IconTrash, IconPencil, IconPlus, IconImage, IconCheck,
  IconImport, IconZoomIn, IconZoomOut, IconDownload, IconSparkles, IconSearch, IconWand,
} from "../utils/icons";
import { t } from "../i18n/index.js";
import TimelineChart from "./TimelineChart.vue";
import ImportSourceModal from "./ImportSourceModal.vue";
import CloudFilePicker from "./CloudFilePicker.vue";
import FilterSidebar from "./FilterSidebar.vue";
import AiToolsSidebar from "./AiToolsSidebar.vue";
import VirtualGrid from "./VirtualGrid.vue";
import LazyImage from "./LazyImage.vue";

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
  sort: { type: Array, default: () => [] },
  formatSize: { type: Function, required: true },
  formatTime: { type: Function, required: true },
  fetchList: { type: Function, required: true },
  loadAllPhotos: { type: Function, default: null },
  collections: { type: Array, default: () => [] },
  selectedCollectionId: { type: String, default: "all" },
  albumPeople: { type: Array, default: () => [] },
  albumPeopleLoading: { type: Boolean, default: false },
  selectedPerson: { type: Object, default: null },
  personPhotos: { type: Array, default: () => [] },
  personPhotosLoading: { type: Boolean, default: false },
  onCreateAlbum: { type: Function, default: null },
  onUpdateAlbum: { type: Function, default: null },
  onShareAlbum: { type: Function, default: null },
  onUnshareAlbum: { type: Function, default: null },
});

const emit = defineEmits([
  "create-album",
  "refresh-albums",
  "select-album",
  "update-album",
  "delete-album",
  "share-album",
  "unshare-album",
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
  "filter-person",
  "refresh-album-people",
  "select-person",
  "back-to-people",
  "patch-person",
  "merge-people",
  "split-person",
]);

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


const isMobile = ref(false);
const filterSortPanelOpen = ref(false);
const aiToolsPanelOpen = ref(false);
const mobileSearchOpen = ref(false);
const mobileSearchInputRef = ref(null);
const thumbnailPopoverOpen = ref(false);
const thumbnailBtnRef = ref(null);
const thumbnailPopoverStyle = ref({});
const THUMB_DEFAULT = 2;
const thumbnailSize = ref(THUMB_DEFAULT);

function toggleThumbnailPopover() {
  thumbnailPopoverOpen.value = !thumbnailPopoverOpen.value;
  if (thumbnailPopoverOpen.value && thumbnailBtnRef.value) {
    const rect = thumbnailBtnRef.value.getBoundingClientRect();
    const pw = 192; // w-48 = 12rem = 192px
    let left = rect.left + rect.width / 2 - pw / 2;
    if (left + pw > window.innerWidth - 8) left = window.innerWidth - 8 - pw;
    if (left < 8) left = 8;
    thumbnailPopoverStyle.value = {
      position: 'fixed',
      top: `${rect.bottom + 4}px`,
      left: `${left}px`,
    };
  }
}

const detailPhoto = ref(null);
const detailExpanded = ref(false);
const detailFullscreen = ref(false);
const detailFullscreenControlsVisible = ref(false);
const detailDimensions = ref(null);
const detailImageFallback = ref(false);
const detailImageEpoch = ref(0);
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

function saveDetailViewPreferences() {
  localStorage.setItem(DETAIL_VIEW_PREFERENCES_KEY, JSON.stringify({
    original: detailOriginalPreference.value,
    darktable: detailDarktablePreference.value,
  }));
}

const selectMode = ref(false);
const selectedPhotoPaths = ref(new Set());
const editRating = ref(0);
const editColorLabels = ref([]);
const editTagsText = ref("");
const imageLoading = ref(false);

// Darktable preview / export
const darktablePreview = ref(false);
const hasXmpSidecar = ref(false);
const showExportDialog = ref(false);
const exporting = ref(false);
const exportForm = ref({ format: "jpeg", quality: 95, maxWidth: 0 });

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
  isMobile.value = window.innerWidth < 768
    || window.screen.width < 768
    || window.navigator.maxTouchPoints > 0
    || window.matchMedia("(pointer: coarse)").matches;
}
onMounted(() => {
  updateMobile();
  filterSortPanelOpen.value = !isMobile.value;
  window.addEventListener("resize", updateMobile);
  window.addEventListener("keydown", onDetailKeydown);
});
onBeforeUnmount(() => {
  window.removeEventListener("resize", updateMobile);
  window.removeEventListener("keydown", onDetailKeydown);
  hideFullscreenControls();
});

const COLOR_LABELS = [
  { key: "red", color: "bg-red-500", ring: "ring-red-400" },
  { key: "yellow", color: "bg-yellow-400", ring: "ring-yellow-300" },
  { key: "green", color: "bg-emerald-500", ring: "ring-emerald-400" },
  { key: "blue", color: "bg-blue-500", ring: "ring-blue-400" },
  { key: "purple", color: "bg-purple-500", ring: "ring-purple-400" },
];

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
    if (props.onCreateAlbum) {
      success = await props.onCreateAlbum({
        name: form.name.trim(),
        description: form.description,
      });
    } else {
      emit("create-album", { name: form.name.trim(), description: form.description });
    }
  } else {
    if (props.onUpdateAlbum) {
      success = await props.onUpdateAlbum({
        dirName: form.dirName,
        name: form.name.trim(),
        description: form.description,
        cover: form.cover,
      });
    } else {
      emit("update-album", {
        dirName: form.dirName,
        name: form.name.trim(),
        description: form.description,
        cover: form.cover,
      });
    }
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
    if (props.onShareAlbum) {
      success = await props.onShareAlbum(payload);
    } else {
      emit("share-album", payload);
    }
    if (success !== false) {
      shareForm.value = { username: "", canWrite: false, canDelete: false };
      syncShareDialogAlbum();
    }
  } finally {
    shareMutationPending.value = false;
  }
}

async function updateSharePermission(share, field, value) {
  if (!shareDialogAlbum.value || shareMutationPending.value) return;
  shareMutationPending.value = true;
  try {
    const payload = {
      dirName: shareDialogAlbum.value.dir_name,
      username: share.username,
      canWrite: field === "can_write" ? value : share.can_write,
      canDelete: field === "can_delete" ? value : share.can_delete,
    };
    if (props.onShareAlbum) {
      await props.onShareAlbum(payload);
      syncShareDialogAlbum();
    } else {
      emit("share-album", payload);
    }
  } finally {
    shareMutationPending.value = false;
  }
}

async function removeShare(album, username) {
  if (shareMutationPending.value) return;
  shareMutationPending.value = true;
  try {
    if (props.onUnshareAlbum) {
      await props.onUnshareAlbum({ dirName: album.dir_name, username });
      syncShareDialogAlbum();
    } else {
      emit("unshare-album", { dirName: album.dir_name, username });
    }
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

const textSearchInput = ref('');

function submitTextSearch() {
  const text = textSearchInput.value.trim();
  if (!text) return;
  emit('search-text', { text, scope: 'current_album' });
}

function clearTextSearch() {
  textSearchInput.value = '';
  emit('show-photos');
}

function toggleFilterSortPanel() {
  filterSortPanelOpen.value = !filterSortPanelOpen.value;
}

function toggleAiToolsPanel() {
  aiToolsPanelOpen.value = !aiToolsPanelOpen.value;
  if (aiToolsPanelOpen.value) {
    emit("refresh-album-people");
    if (isMobile.value) {
      filterSortPanelOpen.value = false;
      mobileSearchOpen.value = false;
    }
  }
}

async function openMobileSearch() {
  mobileSearchOpen.value = true;
  await Promise.resolve();
  mobileSearchInputRef.value?.focus?.();
}

function closeMobileSearch() {
  mobileSearchOpen.value = false;
}

function onAiSearchText(payload) {
  if (payload?.text) {
    textSearchInput.value = payload.text;
  }
  emit('search-text', payload);
}

function enterAlbum(album) {
  emit("select-album", album.dir_name);
}

function exitAlbum() {
  selectMode.value = false;
  selectedPhotoPaths.value.clear();
  emit("select-album", "");
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

async function selectAllPhotos() {
  if (props.loadAllPhotos) {
    await props.loadAllPhotos();
  }
  selectedPhotoPaths.value = new Set(props.albumPhotos.map((p) => p.path));
}

function clearSelection() {
  selectedPhotoPaths.value = new Set();
  selectMode.value = false;
}

function confirmDeleteSelected() {
  const count = selectedPhotoPaths.value.size;
  if (!count) return;
  if (!window.confirm(t('msg.confirmDeletePhotos', { count }))) return;
  emit("delete-photos", [...selectedPhotoPaths.value]);
  selectMode.value = false;
  selectedPhotoPaths.value = new Set();
}

function confirmDeleteSinglePhoto() {
  if (!detailPhoto.value) return;
  if (!window.confirm(t('msg.confirmDeletePhoto', { name: detailPhoto.value.name }))) return;
  emit("delete-photos", [detailPhoto.value.path]);
  detailPhoto.value = null;
  detailExpanded.value = false;
}

function getAlbumCoverThumbnail(album) {
  if (album.cover) {
    const ownerParam = album.owner ? `&owner=${encodeURIComponent(album.owner)}` : "";
    return `/api/thumbnail/${encodeURIComponent(album.cover)}?size=400${ownerParam}`;
  }
  return "";
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

// ── Thumbnail size helpers ──
const THUMB_GRID_CLASSES = [
  "grid-cols-2 gap-1 sm:grid-cols-2",
  "grid-cols-2 gap-1.5 sm:grid-cols-3",
  "grid-cols-3 gap-1.5 sm:grid-cols-4",
  "grid-cols-2 gap-1.5 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 2xl:grid-cols-7",
  "grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-8",
  "grid-cols-4 gap-2 sm:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8 2xl:grid-cols-10",
];

const thumbMaxIdx = THUMB_GRID_CLASSES.length - 1;
const thumbGridClass = computed(() => THUMB_GRID_CLASSES[thumbMaxIdx - thumbnailSize.value] || THUMB_GRID_CLASSES[3]);

// ── Photo grid / detail helpers ──
function handlePhotosScrollNearBottom() {
  if (!props.busyAlbums) {
    emit("load-more-photos");
  }
}

function getThumbnail(path, size = 320, owner = null) {
  const resolvedOwner = owner || currentAlbumOwner.value;
  const ownerParam = resolvedOwner ? `&owner=${encodeURIComponent(resolvedOwner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(path)}?size=${size}${ownerParam}`;
}

const detailIndex = computed(() => {
  if (!detailPhoto.value) return -1;
  return props.albumPhotos.findIndex((p) => p.path === detailPhoto.value.path);
});
const hasPrev = computed(() => detailIndex.value > 0);
const hasNext = computed(() => detailIndex.value >= 0 && detailIndex.value < props.albumPhotos.length - 1);

const detailDirty = computed(() => {
  if (!detailPhoto.value) return false;
  const p = detailPhoto.value;
  if (editRating.value !== (p.rating || 0)) return true;
  const currentLabels = [...editColorLabels.value].sort().join(",");
  const originalLabels = colorLabels(p.color_label).sort().join(",");
  if (currentLabels !== originalLabels) return true;
  const currentTags = editTagsText.value.split(",").map((s) => s.trim()).filter(Boolean).join(", ");
  const originalTags = (p.tags || []).join(", ");
  if (currentTags !== originalTags) return true;
  return false;
});

function beginDetailImageLoad() {
  detailImageEpoch.value += 1;
  detailImageFallback.value = false;
  imageLoading.value = true;
}

function openDetail(photo, expanded = detailOriginalPreference.value) {
  beginDetailImageLoad();
  detailPhoto.value = photo;
  detailExpanded.value = expanded;
  detailDimensions.value = photo.width > 0 && photo.height > 0
    ? `${photo.width} * ${photo.height}`
    : null;
  editRating.value = photo.rating || 0;
  editColorLabels.value = colorLabels(photo.color_label);
  editTagsText.value = (photo.tags || []).join(", ");
  // Restore darktable after XMP check; keep expanded preference immediately.
  darktablePreview.value = false;
  hasXmpSidecar.value = false;
  checkXmpSidecar(photo.path);
}

function closeDetail() {
  if (detailDirty.value && !window.confirm(t('msg.unsavedChangesClose'))) return;
  detailOriginalPreference.value = detailExpanded.value;
  detailDarktablePreference.value = darktablePreview.value;
  saveDetailViewPreferences();
  detailPhoto.value = null;
  detailExpanded.value = false;
  detailFullscreen.value = false;
  hideFullscreenControls();
  detailDimensions.value = null;
  detailImageFallback.value = false;
  darktablePreview.value = false;
  showExportDialog.value = false;
}

function goDetailPrev() {
  if (!hasPrev.value) return;
  if (detailDirty.value && !window.confirm(t('msg.unsavedChangesSwitch'))) return;
  openDetail(props.albumPhotos[detailIndex.value - 1], detailExpanded.value);
}
function goDetailNext() {
  if (!hasNext.value) return;
  if (detailDirty.value && !window.confirm(t('msg.unsavedChangesSwitch'))) return;
  openDetail(props.albumPhotos[detailIndex.value + 1], detailExpanded.value);
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

function onDetailKeydown(e) {
  if (!detailPhoto.value || e.altKey || e.ctrlKey || e.metaKey) return;
  const target = e.target;
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) return;
  if (e.key === "ArrowLeft") {
    e.preventDefault();
    goDetailPrev();
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    goDetailNext();
  }
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
function getOriginalUrl(path) {
  const ownerParam = currentAlbumOwner.value ? `?owner=${encodeURIComponent(currentAlbumOwner.value)}` : "";
  return `/api/download/${encodeURIComponent(path)}${ownerParam}`;
}

function getDarktablePreviewUrl(path, size = 1600) {
  const ownerParam = currentAlbumOwner.value ? `&owner=${encodeURIComponent(currentAlbumOwner.value)}` : "";
  return `/api/albums/darktable_preview/${encodeURIComponent(path)}?size=${size}${ownerParam}`;
}

async function checkXmpSidecar(path) {
  try {
    const ownerParam = currentAlbumOwner.value ? `?owner=${encodeURIComponent(currentAlbumOwner.value)}` : "";
    const res = await fetch(`/api/albums/has_xmp/${encodeURIComponent(path)}${ownerParam}`, { credentials: "same-origin" });
    if (res.ok) {
      const data = await res.json();
      if (!detailPhoto.value || detailPhoto.value.path !== path) return;
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
    hasXmpSidecar.value = false;
  }
}

const RAW_EXTS = new Set(["cr2","cr3","nef","arw","dng","orf","rw2","raf","pef","srw"]);
function isRawFile(name) {
  const ext = (name || "").split(".").pop().toLowerCase();
  return RAW_EXTS.has(ext);
}
const detailImageSrc = computed(() => {
  if (!detailPhoto.value) return "";
  const isRaw = isRawFile(detailPhoto.value.name);

  // Explicit darktable toggle (requires XMP): always use darktable_preview.
  if (darktablePreview.value && hasXmpSidecar.value) {
    const size = detailFullscreen.value || detailExpanded.value ? 0 : 1600;
    return getDarktablePreviewUrl(detailPhoto.value.path, size);
  }

  if (detailImageFallback.value) {
    return getThumbnail(detailPhoto.value.path, 800);
  }

  // Non-darktable: JPEG/etc can use original; RAW goes through /api/thumbnail (LibRaw/lumeno).
  // size=0 requests full-resolution decode (same as darktable_preview).
  if (detailFullscreen.value || detailExpanded.value) {
    if (isRaw) {
      return getThumbnail(detailPhoto.value.path, 0);
    }
    return getOriginalUrl(detailPhoto.value.path);
  }

  return getThumbnail(detailPhoto.value.path, 800);
});

function downloadOriginal() {
  if (!detailPhoto.value) return;
  const a = document.createElement("a");
  a.href = getOriginalUrl(detailPhoto.value.path);
  a.download = "";
  a.click();
}

function openExportDialog() {
  if (!hasXmpSidecar.value) return;
  exportForm.value = { format: "jpeg", quality: 95, maxWidth: 0 };
  showExportDialog.value = true;
}

async function doExport() {
  if (!detailPhoto.value || exporting.value) return;
  exporting.value = true;
  try {
    const body = {
      path: detailPhoto.value.path,
      format: exportForm.value.format,
      quality: exportForm.value.quality,
      max_width: exportForm.value.maxWidth || 0,
    };
    if (currentAlbumOwner.value) body.owner = currentAlbumOwner.value;
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
    const match = disposition.match(/filename="?([^"]+)"?/);
    const filename = match ? match[1] : `export.${exportForm.value.format}`;
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    a.click();
    URL.revokeObjectURL(url);
    showExportDialog.value = false;
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
  const tags = editTagsText.value.split(",").map((s) => s.trim()).filter(Boolean);
  emit("update-photo-meta", {
    path: detailPhoto.value.path,
    tags,
    rating: editRating.value,
    colorLabel: [...editColorLabels.value],
  });
  detailPhoto.value.rating = editRating.value;
  detailPhoto.value.color_label = [...editColorLabels.value];
  detailPhoto.value.tags = tags;
  closeDetail();
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
  if (album.cover) {
    const ownerParam = album.owner ? `&owner=${encodeURIComponent(album.owner)}` : "";
    return `/api/thumbnail/${encodeURIComponent(album.cover)}?size=160${ownerParam}`;
  }
  return "";
}

// Use first photo as cover suggestion when setting cover
function setCoverFromPhoto(photo) {
  if (!currentAlbumObj.value) return;
  emit("update-album", {
    dirName: currentAlbumObj.value.dir_name,
    cover: photo.path,
  });
}
</script>

<template>
  <section
    class="flex h-full min-h-0 flex-col border shadow-[0_22px_60px_rgba(71,43,17,0.16)] backdrop-blur rounded-2xl md:rounded-[2rem]"
    :class="isDark ? 'border-stone-700 bg-stone-900/70 text-stone-100' : 'border-white/70 bg-white/70'"
  >
    <!-- ═══════════════════════════════════════════
         VIEW 1: Album List
         ═══════════════════════════════════════════ -->
    <Transition name="album-view" mode="out-in">
    <div v-if="!inAlbumView" key="album-list" class="flex min-h-0 flex-1 flex-col">
      <!-- Toolbar -->
      <div class="flex items-center gap-1.5 border-b px-2.5 py-2 md:gap-2 md:px-4 md:py-3" :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'">
        <h2 class="text-sm font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">{{ t('albums.title') }}</h2>
        <div class="flex-1"></div>
        <button class="inline-flex items-center gap-1 rounded-2xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
          @click="openCreateDialog">
          <IconPlus class="h-3.5 w-3.5" /><span class="hidden sm:inline">{{ t('albums.create') }}</span>
        </button>
        <button class="inline-flex items-center gap-1 rounded-2xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
          @click="emit('refresh-albums')">
          <IconRefresh class="h-3.5 w-3.5" /><span class="hidden sm:inline">{{ t('albums.refresh') }}</span>
        </button>
      </div>

      <!-- Album grid -->
      <div class="app-scrollbar min-h-0 flex-1 overflow-y-auto p-3 md:p-5">
        <div v-if="busyAlbums && albums.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loading') }}</div>
        <div v-else-if="albums.length === 0" class="flex flex-col items-center justify-center gap-3 py-16">
          <IconImage class="h-12 w-12" :class="isDark ? 'text-stone-600' : 'text-stone-300'" />
          <p class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.empty') }}</p>
        </div>
        <div v-else class="grid grid-cols-1 gap-3 sm:grid-cols-2 md:gap-4 lg:grid-cols-3 xl:grid-cols-4">
          <article v-for="album in albums" :key="album.dir_name"
            class="group cursor-pointer overflow-hidden rounded-xl border transition hover:shadow-lg"
            :class="isDark ? 'border-stone-700 bg-stone-900/80 hover:border-stone-500' : 'border-stone-200 bg-white/80 hover:border-stone-300'"
            @click="enterAlbum(album)">
            <!-- Cover area -->
            <div class="relative aspect-[16/10] overflow-hidden" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
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
              class="group cursor-pointer overflow-hidden rounded-xl border transition hover:shadow-lg"
              :class="isDark ? 'border-blue-800/40 bg-stone-900/80 hover:border-blue-600/60' : 'border-blue-200 bg-white/80 hover:border-blue-300'"
              @click="enterAlbum(album)">
              <div class="relative aspect-[16/10] overflow-hidden" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
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
    <div v-else key="photo-browser" class="flex min-h-0 flex-1 flex-col">
      <!-- Top toolbar -->
      <div class="flex items-center gap-1.5 border-b px-2.5 py-2 md:gap-2 md:px-4 md:py-3" :class="isDark ? 'border-stone-700/60' : 'border-stone-200/60'">
        <button class="inline-flex items-center gap-1 rounded-2xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
          @click="exitAlbum">
          <IconArrowLeft class="h-3.5 w-3.5" /><span class="hidden sm:inline">{{ t('albums.back') }}</span>
        </button>
        <h2 class="min-w-0 truncate text-sm font-bold" :class="isDark ? 'text-stone-100' : 'text-stone-800'">
          {{ currentAlbumObj?.name || currentAlbum }}
          <span v-if="isSharedAlbum" class="ml-1 text-xs font-normal" :class="isDark ? 'text-blue-400' : 'text-blue-600'">
            (​{{ t('albums.from', { owner: currentAlbumOwner }) }})
          </span>
        </h2>
        <div class="flex-1"></div>
        <!-- Desktop: full search field -->
        <div v-if="!inSpecialView && !isMobile" class="flex min-w-0 max-w-xs flex-1 items-center gap-1 lg:max-w-sm">
          <input
            v-model="textSearchInput"
            type="search"
            :placeholder="t('albums.textSearchPlaceholder')"
            class="min-w-0 flex-1 rounded-xl border px-2.5 py-1.5 text-xs"
            :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
            @keydown.enter.prevent="submitTextSearch"
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
            v-if="textSearchInput"
            class="inline-flex shrink-0 items-center rounded-xl px-2 py-1.5 text-xs transition"
            :class="isDark ? 'bg-stone-800 text-stone-300 hover:bg-stone-700' : 'bg-stone-200 text-stone-600 hover:bg-stone-300'"
            :title="t('albums.clear')"
            @click="clearTextSearch"
          >
            <IconClose class="h-3.5 w-3.5" />
          </button>
        </div>
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
              type="search"
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
        <button v-if="!isSharedAlbum && !inSpecialView && !(isMobile && mobileSearchOpen)" class="inline-flex items-center gap-1 rounded-2xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="isDark ? 'bg-emerald-700 text-white hover:bg-emerald-600' : 'bg-emerald-600 text-white hover:bg-emerald-700'"
          @click="openImportModal">
          <IconImport class="h-3.5 w-3.5" /><span class="hidden sm:inline">{{ t('albums.import') }}</span>
        </button>
        <!-- Hidden file input for local import -->
        <input ref="localFileInput" type="file" class="hidden" multiple accept="image/*,video/*,.raw,.arw,.nef,.cr2,.cr3,.orf,.rw2,.dng,.raf,.srw,.x3f,.3fr,.mef,.pef,.nrw,.rwl,.iiq,.erf,.fff" @change="handleLocalFileChange" />
        <!-- Import source picker modal -->
        <ImportSourceModal
          v-if="importSourceModalOpen"
          :is-dark="isDark"
          @local="handleImportLocal"
          @cloud="handleImportCloud"
          @close="importSourceModalOpen = false"
        />
        <!-- Cloud file picker -->
        <CloudFilePicker
          v-if="cloudPickerOpen"
          :is-dark="isDark"
          :fetch-list="fetchList"
          @confirm="handleCloudPickerConfirm"
          @close="cloudPickerOpen = false"
        />
        <button v-if="currentAlbumCanDelete && !inSpecialView && !(isMobile && mobileSearchOpen)" class="inline-flex items-center gap-1 rounded-2xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="selectMode
            ? (isDark ? 'bg-sky-600 text-white hover:bg-sky-500' : 'bg-sky-500 text-white hover:bg-sky-600')
            : (isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300')"
          @click="toggleSelectMode">
          <IconCheck class="h-3.5 w-3.5" /><span class="hidden sm:inline">{{ t('albums.select') }}</span>
        </button>
        <button v-if="!(isMobile && mobileSearchOpen)" class="inline-flex items-center gap-1 rounded-2xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="isDark ? 'bg-stone-800 text-stone-100 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
          @click="emit('rebuild-index')">
          <IconDatabase class="h-3.5 w-3.5" /><span class="hidden sm:inline">{{ t('albums.rebuildIndex') }}</span>
        </button>
        <button
          v-if="inSpecialView"
          class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900'"
          @click="emit('show-photos')"
        >
          <IconImage class="h-3.5 w-3.5" />
          <span class="hidden sm:inline">{{ t('albums.backToPhotos') }}</span>
        </button>
        <button v-if="!inSpecialView && !(isMobile && mobileSearchOpen)" class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="filterSortPanelOpen
            ? (isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900')
            : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')"
          @click="toggleFilterSortPanel">
          <IconFilter class="h-3.5 w-3.5" />
          <span class="hidden sm:inline">{{ t('albums.filterAndSort') }}</span>
          <span v-if="hasActiveFilters || hasActiveSort" class="ml-0.5 h-1.5 w-1.5 rounded-full bg-amber-500"></span>
        </button>
        <button v-if="!(isMobile && mobileSearchOpen)" class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
          :class="aiToolsPanelOpen || inSpecialView
            ? (isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900')
            : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')"
          @click="toggleAiToolsPanel">
          <IconWand class="h-3.5 w-3.5" />
          <span class="hidden sm:inline">{{ t('albums.aiTools') }}</span>
        </button>
        <div v-if="!inSpecialView && !(isMobile && mobileSearchOpen)" class="relative">
          <button ref="thumbnailBtnRef" class="inline-flex items-center gap-1 rounded-xl px-2.5 py-1.5 text-xs font-medium transition md:gap-1.5 md:px-3 md:py-2"
            :class="thumbnailPopoverOpen
              ? (isDark ? 'bg-violet-500/20 text-violet-200' : 'bg-violet-100 text-violet-900')
              : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')"
            @click="toggleThumbnailPopover">
            <IconZoomIn class="h-3.5 w-3.5" />
            <span class="hidden sm:inline">{{ t('albums.thumbnailSize') }}</span>
          </button>
          <teleport to="body">
            <div v-if="thumbnailPopoverOpen" class="fixed inset-0 z-40" @click="thumbnailPopoverOpen = false" @wheel.prevent="thumbnailPopoverOpen = false">
              <div class="z-50 w-48 rounded-xl border p-3 shadow-lg"
                :style="thumbnailPopoverStyle"
                :class="isDark ? 'border-stone-700 bg-stone-900' : 'border-stone-200 bg-white'"
                @click.stop @wheel.stop>
                <div class="flex items-center gap-1.5">
                  <IconZoomOut class="h-3.5 w-3.5 shrink-0" :class="isDark ? 'text-stone-400' : 'text-stone-500'" />
                  <input type="range" min="0" :max="thumbMaxIdx" v-model.number="thumbnailSize"
                    class="w-full accent-violet-500" />
                  <IconZoomIn class="h-3.5 w-3.5 shrink-0" :class="isDark ? 'text-stone-400' : 'text-stone-500'" />
                  <button class="shrink-0 rounded p-0.5 transition"
                    :class="isDark ? 'text-stone-400 hover:text-stone-200 hover:bg-stone-700' : 'text-stone-400 hover:text-stone-600 hover:bg-stone-200'"
                    @click="thumbnailSize = THUMB_DEFAULT">
                    <IconRefresh class="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            </div>
          </teleport>
        </div>
      </div>

      <!-- Selection action bar -->
      <div v-if="selectMode && !inSpecialView" class="flex items-center gap-2 border-b px-2.5 py-1.5 md:px-4"
        :class="isDark ? 'border-stone-700/60 bg-stone-800/80' : 'border-stone-200/60 bg-stone-100/80'">
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
          class="inline-flex items-center gap-1 rounded-xl px-3 py-1.5 text-xs font-medium transition"
          :class="isDark ? 'bg-rose-700 text-white hover:bg-rose-600' : 'bg-rose-600 text-white hover:bg-rose-700'"
          @click="confirmDeleteSelected">
          <IconTrash class="h-3.5 w-3.5" />{{ t('albums.deleteSelected') }}
        </button>
      </div>

      <!-- Main content: filter/sort sidebar + photo grid + AI tools -->
      <div class="flex min-h-0 flex-1 overflow-hidden">
        <!-- Desktop filter/sort sidebar -->
        <FilterSidebar
          v-if="!isMobile && filterSortPanelOpen"
          v-show="!inSpecialView"
          :is-dark="isDark"
          :is-mobile="false"
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

        <!-- Mobile filter/sort drawer -->
        <teleport to="body">
          <template v-if="isMobile">
            <Transition name="drawer-backdrop" appear>
              <div
                v-show="filterSortPanelOpen && !inSpecialView"
                class="fixed inset-0 z-[90] bg-black/50 backdrop-blur-sm"
                style="position: fixed; inset: 0; z-index: 90;"
                @click="filterSortPanelOpen = false"
              ></div>
            </Transition>
            <Transition name="mobile-drawer" appear>
              <FilterSidebar
                v-show="filterSortPanelOpen && !inSpecialView"
                style="position: fixed; inset: 0 auto 0 0; z-index: 91; width: min(18rem, 85vw);"
                :is-dark="isDark"
                :is-mobile="true"
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
            </Transition>
          </template>
        </teleport>

        <!-- Photo grid / duplicate groups -->
        <div class="relative min-h-0 flex-1 overflow-hidden p-2 md:p-3">
          <div v-if="inDuplicatesView" class="app-scrollbar absolute inset-0 overflow-y-auto p-2 md:p-3">
            <div v-if="busyAlbums && duplicateGroups.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loadingAlbum') }}</div>
            <div v-else-if="!busyAlbums && duplicateGroups.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.noDuplicateGroups') }}</div>
            <div v-else class="flex flex-col gap-4 md:gap-5">
              <div class="flex items-center justify-between px-1">
                <p class="text-xs font-semibold uppercase tracking-wide" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.showDuplicates') }}</p>
                <p class="text-xs" :class="isDark ? 'text-stone-500' : 'text-stone-400'">{{ duplicateTotal }}</p>
              </div>
              <section v-for="group in duplicateGroups" :key="group.group_key"
                class="rounded-2xl border p-3 md:p-4"
                :class="isDark ? 'border-stone-700 bg-stone-900/70' : 'border-stone-200 bg-white/80'">
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
                <div class="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
                  <article v-for="photo in group.photos" :key="photo.stable_id"
                    class="group cursor-pointer rounded-xl border p-1.5 transition hover:shadow-md sm:p-2"
                    :class="isDark ? 'border-stone-700 bg-stone-900/80 hover:border-stone-600' : 'border-stone-200 bg-white/80 hover:border-stone-300'"
                    @click="openDuplicateDetail(photo)">
                    <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                      <LazyImage img-class="h-full w-full object-cover transition group-hover:scale-105" :src="getThumbnail(photo.path, 360)" alt="" />
                    </div>
                    <p class="mt-1.5 truncate text-xs font-semibold" :title="duplicateDisplayName(photo)">{{ duplicateDisplayName(photo) }}</p>
                    <p class="truncate text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ formatSize(photo.size_bytes) }}</p>
                  </article>
                </div>
              </section>
            </div>
          </div>

          <div v-else-if="inTextView" class="app-scrollbar absolute inset-0 overflow-y-auto p-2 md:p-3">
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
            <div v-else class="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
              <article v-for="photo in similarResults" :key="`${photo.owner}:${photo.path}`"
                class="group cursor-pointer rounded-xl border p-1.5 transition hover:shadow-md sm:p-2"
                :class="isDark ? 'border-stone-700 bg-stone-900/80 hover:border-stone-600' : 'border-stone-200 bg-white/80 hover:border-stone-300'"
                @click="openSimilarDetail(photo)">
                <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                  <LazyImage img-class="h-full w-full object-cover transition group-hover:scale-105" :src="getThumbnail(photo.path, 360, photo.owner)" alt="" />
                </div>
                <p class="mt-1.5 truncate text-[11px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ photo.name }}</p>
                <p class="truncate text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  {{ t('albums.similarScore', { score: Number(photo.score || 0).toFixed(2) }) }}
                </p>
              </article>
            </div>
          </div>

          <div v-else-if="inSimilarView" class="app-scrollbar absolute inset-0 overflow-y-auto p-2 md:p-3">
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
            <div v-else class="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
              <article v-for="photo in similarResults" :key="`${photo.owner}:${photo.path}`"
                class="group cursor-pointer rounded-xl border p-1.5 transition hover:shadow-md sm:p-2"
                :class="isDark ? 'border-stone-700 bg-stone-900/80 hover:border-stone-600' : 'border-stone-200 bg-white/80 hover:border-stone-300'"
                @click="openSimilarDetail(photo)">
                <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                  <LazyImage img-class="h-full w-full object-cover transition group-hover:scale-105" :src="getThumbnail(photo.path, 360, photo.owner)" alt="" />
                </div>
                <p class="mt-1.5 truncate text-[11px] font-medium" :class="isDark ? 'text-stone-200' : 'text-stone-700'">{{ photo.name }}</p>
                <p class="truncate text-[10px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">
                  {{ t('albums.similarScore', { score: Number(photo.score || 0).toFixed(2) }) }}
                </p>
              </article>
            </div>
          </div>

          <template v-else>
          <div v-if="busyAlbums && albumPhotos.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.loadingAlbum') }}</div>
          <div v-else-if="!busyAlbums && albumPhotos.length === 0" class="py-10 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('albums.noPhotosFiltered') }}</div>

          <VirtualGrid
            v-else
            :items="albumPhotos"
            item-key="path"
            :grid-class="'grid ' + thumbGridClass"
            class="absolute inset-0"
            @scroll-near-bottom="handlePhotosScrollNearBottom"
          >
            <template #default="{ item: photo }">
            <article :key="photo.path"
              class="group cursor-pointer rounded-xl border p-1.5 transition hover:shadow-md sm:p-2"
              :class="[
                selectMode && selectedPhotoPaths.has(photo.path)
                  ? (isDark ? 'border-sky-500 bg-sky-900/30 ring-1 ring-sky-500/50' : 'border-sky-400 bg-sky-50 ring-1 ring-sky-400/50')
                  : (isDark ? 'border-stone-700 bg-stone-900/80 hover:border-stone-600' : 'border-stone-200 bg-white/80 hover:border-stone-300'),
              ]"
              @click="selectMode ? togglePhotoSelect(photo) : openDetail(photo)">
              <div class="relative aspect-square overflow-hidden rounded-lg" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
                <LazyImage
                  img-class="h-full w-full object-cover transition group-hover:scale-105"
                  :src="getThumbnail(photo.path, 360)"
                  alt="" />
                <!-- Selection checkbox -->
                <div v-if="selectMode"
                  class="absolute right-1.5 top-1.5 flex h-5 w-5 items-center justify-center rounded-full border-2 transition"
                  :class="selectedPhotoPaths.has(photo.path)
                    ? 'border-sky-400 bg-sky-500 text-white'
                    : (isDark ? 'border-stone-400 bg-stone-900/60' : 'border-stone-400 bg-white/80')">
                  <IconCheck v-if="selectedPhotoPaths.has(photo.path)" class="h-3 w-3" />
                </div>
                <template v-if="photo.color_label">
                  <span
                    v-for="(clKey, ci) in colorLabels(photo.color_label)"
                    :key="clKey"
                    class="absolute top-1.5 h-2.5 w-2.5 rounded-full ring-1 ring-white/50"
                    :class="photoColorDot(clKey) ? photoColorDot(clKey).color : ''"
                    :style="`left: ${0.375 + ci * 0.875}rem`"
                  ></span>
                </template>
                <div v-if="photo.rating > 0" class="absolute bottom-1 left-1 flex">
                  <IconStar v-for="s in photo.rating" :key="s" class="h-3 w-3 text-amber-400 drop-shadow" />
                </div>
              </div>
              <p class="mt-1.5 truncate text-xs font-semibold" :title="photo.name">{{ photo.name }}</p>
              <p class="truncate text-[10px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">
                {{ photo.datetime || photo.date }}
                <template v-if="photo.camera_model"> &middot; {{ photo.camera_model }}</template>
              </p>
              <div v-if="photo.tags && photo.tags.length > 0" class="mt-0.5 flex flex-wrap gap-0.5">
                <span v-for="tag in photo.tags.slice(0, 3)" :key="tag"
                  class="rounded px-1 py-0 text-[9px]"
                  :class="isDark ? 'bg-sky-900/50 text-sky-200' : 'bg-sky-50 text-sky-600'">{{ tag }}</span>
                <span v-if="photo.tags.length > 3" class="text-[9px]" :class="isDark ? 'text-stone-500' : 'text-stone-400'">+{{ photo.tags.length - 3 }}</span>
              </div>
            </article>
            </template>
          </VirtualGrid>
          </template>
        </div>

        <!-- Desktop AI tools sidebar -->
        <AiToolsSidebar
          v-if="aiToolsPanelOpen && !isMobile"
          :is-dark="isDark"
          :is-mobile="false"
          :album-view-mode="albumViewMode"
          :initial-query="textSearchInput"
          :similar-loading="similarLoading"
          :people="albumPeople"
          :people-loading="albumPeopleLoading"
          :selected-person="selectedPerson"
          :person-photos="personPhotos"
          :person-photos-loading="personPhotosLoading"
          @close="aiToolsPanelOpen = false"
          @search-text="onAiSearchText"
          @show-duplicates="emit('show-duplicates')"
          @show-photos="emit('show-photos')"
          @filter-person="(person) => emit('filter-person', person)"
          @refresh-people="emit('refresh-album-people')"
          @select-person="(person) => emit('select-person', person)"
          @back-to-people="emit('back-to-people')"
          @patch-person="(id, updates) => emit('patch-person', id, updates)"
          @merge-people="(body) => emit('merge-people', body)"
          @split-person="(body) => emit('split-person', body)"
        />

        <!-- Mobile AI tools drawer (from right) -->
        <teleport to="body">
          <template v-if="isMobile">
            <Transition name="drawer-backdrop" appear>
              <div
                v-show="aiToolsPanelOpen"
                class="fixed inset-0 z-[90] bg-black/50 backdrop-blur-sm"
                @click="aiToolsPanelOpen = false"
              ></div>
            </Transition>
            <Transition name="mobile-drawer-right" appear>
              <AiToolsSidebar
                v-show="aiToolsPanelOpen"
                style="position: fixed; inset: 0 0 0 auto; z-index: 91; width: min(20rem, 90vw);"
                :is-dark="isDark"
                :is-mobile="true"
                :album-view-mode="albumViewMode"
                :initial-query="textSearchInput"
                :similar-loading="similarLoading"
                :people="albumPeople"
                :people-loading="albumPeopleLoading"
                :selected-person="selectedPerson"
                :person-photos="personPhotos"
                :person-photos-loading="personPhotosLoading"
                @close="aiToolsPanelOpen = false"
                @search-text="onAiSearchText"
                @show-duplicates="emit('show-duplicates')"
                @show-photos="emit('show-photos')"
                @filter-person="(person) => emit('filter-person', person)"
                @refresh-people="emit('refresh-album-people')"
                @select-person="(person) => emit('select-person', person)"
                @back-to-people="emit('back-to-people')"
                @patch-person="(id, updates) => emit('patch-person', id, updates)"
                @merge-people="(body) => emit('merge-people', body)"
                @split-person="(body) => emit('split-person', body)"
              />
            </Transition>
          </template>
        </teleport>
      </div>
    </div>
    </Transition>

    <!-- ═══════════════════════════════════════════
         Album Create / Edit Dialog
         ═══════════════════════════════════════════ -->
    <teleport to="body">
      <div v-if="showAlbumDialog" class="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm" @click.self="showAlbumDialog = false">
        <div class="w-full max-w-md rounded-2xl border p-5 shadow-2xl"
          :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100' : 'border-stone-200 bg-white text-stone-900'">
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
      </div>
    </teleport>

    <!-- ═══════════════════════════════════════════
         Share Dialog
         ═══════════════════════════════════════════ -->
    <teleport to="body">
      <div v-if="showShareDialog" class="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm" @click.self="showShareDialog = false">
        <div class="w-full max-w-lg rounded-2xl border p-5 shadow-2xl"
          :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100' : 'border-stone-200 bg-white text-stone-900'">
          <h3 class="mb-4 text-base font-bold">{{ t('share.title', { name: shareDialogAlbum?.name || shareDialogAlbum?.dir_name }) }}</h3>

          <!-- Shares table -->
          <div v-if="shareDialogAlbum?.shares?.length" class="mb-4 overflow-hidden rounded-lg border"
            :class="isDark ? 'border-stone-700' : 'border-stone-200'">
            <table class="w-full text-sm">
              <thead>
                <tr :class="isDark ? 'bg-stone-800 text-stone-400' : 'bg-stone-50 text-stone-500'">
                  <th class="px-3 py-2 text-left text-xs font-medium">{{ t('share.user') }}</th>
                  <th class="px-3 py-2 text-center text-xs font-medium">{{ t('share.canEdit') }}</th>
                  <th class="px-3 py-2 text-center text-xs font-medium">{{ t('share.canDelete') }}</th>
                  <th class="w-10 px-3 py-2"></th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="s in shareDialogAlbum.shares" :key="s.username"
                  class="border-t" :class="isDark ? 'border-stone-700' : 'border-stone-200'">
                  <td class="px-3 py-2 font-medium">{{ s.username }}</td>
                  <td class="px-3 py-2 text-center">
                    <input type="checkbox" :checked="s.can_write"
                      class="h-4 w-4 rounded border-stone-300"
                      :disabled="shareMutationPending"
                      @change="updateSharePermission(s, 'can_write', $event.target.checked)" />
                  </td>
                  <td class="px-3 py-2 text-center">
                    <input type="checkbox" :checked="s.can_delete"
                      class="h-4 w-4 rounded border-stone-300"
                      :disabled="shareMutationPending"
                      @change="updateSharePermission(s, 'can_delete', $event.target.checked)" />
                  </td>
                  <td class="px-3 py-2 text-center">
                    <button class="rounded p-1 transition"
                      :class="isDark ? 'text-rose-400 hover:bg-rose-900/30' : 'text-rose-500 hover:bg-rose-100'"
                      :disabled="shareMutationPending"
                      @click="removeShare(shareDialogAlbum, s.username)">
                      <IconClose class="h-3.5 w-3.5" />
                    </button>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          <div v-else class="mb-4 rounded-lg px-3 py-4 text-center text-sm"
            :class="isDark ? 'bg-stone-800 text-stone-400' : 'bg-stone-50 text-stone-500'">
            {{ t('share.noShares') }}
          </div>

          <!-- Add new share row -->
          <div class="flex items-end gap-2">
            <div class="flex-1">
              <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('share.addUser') }}</label>
              <input v-model="shareForm.username" type="text" :placeholder="t('share.usernamePlaceholder')"
                class="w-full rounded-lg border px-3 py-2 text-sm"
                :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'"
                :disabled="shareMutationPending"
                @keyup.enter="submitShare" />
            </div>
            <label class="flex items-center gap-1 pb-2 text-xs">
              <input type="checkbox" v-model="shareForm.canWrite" class="rounded border-stone-300" :disabled="shareMutationPending" />
              <span :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('share.write') }}</span>
            </label>
            <label class="flex items-center gap-1 pb-2 text-xs">
              <input type="checkbox" v-model="shareForm.canDelete" class="rounded border-stone-300" :disabled="shareMutationPending" />
              <span :class="isDark ? 'text-stone-300' : 'text-stone-600'">{{ t('share.delete') }}</span>
            </label>
            <button class="shrink-0 rounded-lg px-3 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-blue-600 text-white hover:bg-blue-500' : 'bg-blue-600 text-white hover:bg-blue-700'"
              :disabled="shareMutationPending"
              @click="submitShare">
              <IconPlus class="inline h-4 w-4" />
            </button>
          </div>

          <div class="mt-4 flex justify-end">
            <button class="rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
              @click="showShareDialog = false">{{ t('share.close') }}</button>
          </div>
        </div>
      </div>
    </teleport>

    <!-- ═══════════════════════════════════════════
         Photo Detail / Edit overlay
         ═══════════════════════════════════════════ -->
    <teleport to="body">
      <div v-if="detailPhoto" class="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 backdrop-blur-sm" @click.self="closeDetail">
        <div class="relative flex overflow-hidden border shadow-2xl"
          :class="[
            detailFullscreen
              ? 'h-full w-full flex-col border-none bg-black text-white'
              : [
                  isDark ? 'border-stone-700 bg-stone-900 text-stone-100' : 'border-stone-200 bg-white text-stone-900',
                  isMobile ? 'h-full w-full flex-col' : 'w-full flex-col rounded-2xl',
                  detailExpanded && !isMobile ? 'md:mx-auto md:h-[80vh] md:w-[80vw] md:rounded-2xl' : 'md:mx-4 md:max-h-[90vh] md:max-w-3xl md:rounded-2xl'
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
              v-if="detailIndex >= 0">{{ detailIndex + 1 }} / {{ albumPhotos.length }}</span>
            <div class="flex-1"></div>
            <button class="rounded-full p-1.5 transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              :title="t('detail.fullscreen')"
              @click="toggleDetailFullscreen">
              <IconExpand class="h-4 w-4" />
            </button>
            <button class="inline-flex items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium transition"
              :class="detailExpanded
                ? (isDark ? 'bg-amber-500/20 text-amber-200' : 'bg-amber-100 text-amber-900')
                : (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')"
              @click="toggleExpanded">
              <IconExpand class="h-3.5 w-3.5" />
              <span class="hidden sm:inline">{{ detailExpanded ? t('detail.collapse') : t('detail.viewOriginal') }}</span>
            </button>
            <button class="inline-flex items-center gap-1 rounded-full px-2.5 py-1.5 text-xs font-medium transition"
              :class="darktablePreview
                ? (isDark ? 'bg-emerald-500/20 text-emerald-200' : 'bg-emerald-100 text-emerald-900')
                : hasXmpSidecar
                  ? (isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300')
                  : 'opacity-30 cursor-default'"
              :disabled="!hasXmpSidecar"
              :title="!hasXmpSidecar ? t('detail.noXmp') : t('detail.darktablePreview')"
              @click="toggleDarktablePreview">
              <IconCamera class="h-3.5 w-3.5" />
              <span class="hidden sm:inline">{{ t('detail.darktablePreview') }}</span>
            </button>
            <button class="rounded-full p-1.5 transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-700 hover:bg-stone-300'"
              @click="closeDetail">
              <IconClose class="h-4 w-4" />
            </button>
          </div>

          <div class="flex min-h-0 flex-1 flex-col overflow-y-auto p-4 md:flex-row md:gap-5 md:overflow-hidden md:p-5"
            :class="detailFullscreen ? '!overflow-hidden !p-0' : ''">
            <!-- Preview -->
            <div class="group relative flex min-w-0 flex-1 items-center justify-center"
              :class="detailFullscreen ? 'h-full w-full' : ''"
              @mousemove="showFullscreenControls"
              @wheel.prevent="onDetailWheel">
              <!-- Loading spinner -->
              <div v-if="imageLoading" class="absolute inset-0 z-10 flex items-center justify-center">
                <svg class="h-8 w-8 animate-spin" :class="isDark ? 'text-stone-400' : 'text-stone-500'" xmlns="http://www.w3.org/2000/svg" fill="none" viewBox="0 0 24 24">
                  <circle class="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" stroke-width="4"></circle>
                  <path class="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"></path>
                </svg>
              </div>
              <img :key="detailImageEpoch"
                :src="detailImageSrc"
                class="rounded-xl object-contain transition-opacity"
                :class="[
                  detailFullscreen ? 'h-full max-h-full w-full max-w-full rounded-none' : (detailExpanded ? 'max-h-64 sm:max-h-[60vh] md:max-h-full' : 'max-h-48 sm:max-h-72 md:max-h-[60vh]'),
                  imageLoading ? 'opacity-40' : 'opacity-100'
                ]"
                alt="" @load="onDetailImageLoad" @error="onDetailImageError" />

              <div v-if="detailFullscreen"
                class="pointer-events-none absolute left-4 top-4 max-w-[calc(100%-10rem)] rounded-md bg-black/40 px-3 py-2 text-white backdrop-blur-sm transition-opacity"
                :class="detailFullscreenControlsVisible ? 'opacity-100' : 'opacity-0'">
                <p v-if="detailIndex >= 0" class="text-xs tabular-nums text-white/80">{{ detailIndex + 1 }} / {{ albumPhotos.length }}</p>
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
                  <IconExpand class="h-5 w-5" />
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
            <div v-if="!detailFullscreen" class="app-scrollbar mt-3 flex w-full shrink-0 flex-col gap-2.5 overflow-y-auto md:mt-0 md:w-72 md:gap-3 lg:w-80">
              <h3 class="text-base font-bold">{{ detailPhoto.name }}</h3>

              <div class="grid grid-cols-2 gap-x-4 gap-y-1 text-xs" :class="isDark ? 'text-stone-300' : 'text-stone-600'">
                <div v-if="detailPhoto.datetime || detailPhoto.date"><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.date') }}</span> {{ detailPhoto.datetime || detailPhoto.date }}</div>
                <div v-if="detailPhoto.camera_make || detailPhoto.camera_model">
                  <span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.camera') }}</span>
                  {{ [detailPhoto.camera_make, detailPhoto.camera_model].filter(Boolean).join(' ') }}
                </div>
                <div v-if="detailPhoto.lens"><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.lens') }}</span> {{ detailPhoto.lens }}</div>
                <div v-if="detailPhoto.focal_length"><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.focalLength') }}</span> {{ detailPhoto.focal_length }}</div>
                <div v-if="detailPhoto.aperture"><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.aperture') }}</span> {{ detailPhoto.aperture }}</div>
                <div v-if="detailPhoto.shutter_speed"><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.shutter') }}</span> {{ detailPhoto.shutter_speed }}</div>
                <div v-if="detailPhoto.iso"><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">ISO</span> {{ detailPhoto.iso }}</div>
                <div><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.size') }}</span> {{ formatSize(detailPhoto.size) }}</div>
                <div v-if="detailDimensions"><span class="font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.dimensions') }}</span> {{ detailDimensions }}</div>
              </div>

              <hr :class="isDark ? 'border-stone-700' : 'border-stone-200'" />

              <!-- Download buttons -->
              <div class="flex gap-2">
                <button class="inline-flex flex-1 items-center justify-center gap-1.5 rounded-xl px-3 py-2 text-xs font-medium transition"
                  :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
                  @click="downloadOriginal">
                  <IconDownload class="h-3.5 w-3.5" />{{ t('detail.downloadOriginal') }}
                </button>
                <button class="inline-flex flex-1 items-center justify-center gap-1.5 rounded-xl px-3 py-2 text-xs font-medium transition"
                  :class="hasXmpSidecar
                    ? (isDark ? 'bg-emerald-700 text-emerald-100 hover:bg-emerald-600' : 'bg-emerald-600 text-white hover:bg-emerald-700')
                    : 'opacity-40 cursor-default'"
                  :disabled="!hasXmpSidecar"
                  :title="!hasXmpSidecar ? t('detail.noXmp') : ''"
                  @click="openExportDialog">
                  <IconDownload class="h-3.5 w-3.5" />{{ t('detail.downloadEdited') }}
                </button>
              </div>

              <!-- Rating editor -->
              <div>
                <p class="mb-1 text-[10px] font-semibold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.rating') }}</p>
                <div class="flex gap-1">
                  <button v-for="star in 5" :key="star" class="p-0.5 transition"
                    :class="star <= editRating ? 'text-amber-400' : (isDark ? 'text-stone-600 hover:text-stone-400' : 'text-stone-300 hover:text-stone-400')"
                    @click="setDetailRating(star)">
                    <IconStar class="h-5 w-5" />
                  </button>
                </div>
              </div>

              <!-- Color label editor -->
              <div>
                <p class="mb-1 text-[10px] font-semibold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.colorLabel') }}</p>
                <div class="flex gap-2">
                  <button v-for="cl in COLOR_LABELS" :key="cl.key"
                    class="h-6 w-6 rounded-full transition"
                    :class="[cl.color, editColorLabels.includes(cl.key) ? `ring-2 ${cl.ring} ring-offset-1` : 'opacity-50 hover:opacity-100']"
                    :style="isDark && editColorLabels.includes(cl.key) ? 'ring-offset-color: #1c1917' : ''"
                    @click="setDetailColorLabel(cl.key)"></button>
                </div>
              </div>

              <!-- Tags editor -->
              <div>
                <p class="mb-1 text-[10px] font-semibold uppercase tracking-wider" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.tags') }}</p>
                <input v-model="editTagsText" type="text" :placeholder="t('detail.tagsPlaceholder')"
                  class="w-full rounded-lg border px-2.5 py-1.5 text-xs"
                  :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
              </div>

              <!-- Set as album cover + Add to album + Delete -->
              <div class="flex flex-wrap gap-2">
                <button class="flex-1 rounded-xl px-4 py-2 text-xs font-medium transition"
                  :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
                  @click="setCoverFromPhoto(detailPhoto)">
                  {{ t('detail.setAsCover') }}
                </button>
                <button
                  class="inline-flex items-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
                  :class="isDark ? 'bg-sky-900/60 text-sky-200 hover:bg-sky-800' : 'bg-sky-100 text-sky-700 hover:bg-sky-200'"
                  @click="openAddToAlbumModal(detailPhoto)">
                  <IconImage class="h-3.5 w-3.5" />{{ t('detail.addToAlbum') }}
                </button>
                <button
                  class="inline-flex items-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
                  :class="isDark ? 'bg-violet-900/60 text-violet-200 hover:bg-violet-800' : 'bg-violet-100 text-violet-700 hover:bg-violet-200'"
                  :title="t('detail.findSimilar')"
                  @click="findSimilarFromDetail">
                  <IconSparkles class="h-3.5 w-3.5" />{{ t('detail.findSimilar') }}
                </button>
                <button v-if="currentAlbumCanDelete"
                  class="inline-flex items-center gap-1 rounded-xl px-4 py-2 text-xs font-medium transition"
                  :class="isDark ? 'bg-rose-900/60 text-rose-200 hover:bg-rose-800' : 'bg-rose-100 text-rose-700 hover:bg-rose-200'"
                  @click="confirmDeleteSinglePhoto">
                  <IconTrash class="h-3.5 w-3.5" />{{ t('detail.delete') }}
                </button>
              </div>

              <button class="mt-auto rounded-xl px-4 py-2.5 text-sm font-medium transition md:py-2"
                :class="isDark ? 'bg-stone-100 text-stone-900 hover:bg-white' : 'bg-stone-900 text-white hover:bg-stone-800'"
                @click="savePhotoMeta">
                {{ t('detail.save') }}
              </button>
              <div class="h-[env(safe-area-inset-bottom,0px)] md:hidden"></div>
            </div>
          </div>
        </div>
      </div>
    </teleport>

    <!-- ═══════════════════════════════════════════
         Export Dialog
         ═══════════════════════════════════════════ -->
    <teleport to="body">
      <div v-if="showExportDialog" class="fixed inset-0 z-[110] flex items-center justify-center bg-black/50 backdrop-blur-sm" @click.self="showExportDialog = false">
        <div class="w-full max-w-sm rounded-2xl border p-5 shadow-2xl"
          :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100' : 'border-stone-200 bg-white text-stone-900'">
          <h3 class="mb-4 text-base font-bold">{{ t('detail.exportTitle') }}</h3>

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
            <input type="range" v-model.number="exportForm.quality" min="1" max="100" class="w-full" />
          </div>

          <!-- Max width -->
          <div class="mb-4">
            <label class="mb-1 block text-xs font-medium" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('detail.exportMaxWidth') }}</label>
            <input type="number" v-model.number="exportForm.maxWidth" min="0" step="100"
              :placeholder="t('detail.exportMaxWidthPlaceholder')"
              class="w-full rounded-lg border px-2.5 py-1.5 text-sm"
              :class="isDark ? 'border-stone-700 bg-stone-800 text-stone-100 placeholder:text-stone-500' : 'border-stone-300 bg-white text-stone-900 placeholder:text-stone-400'" />
          </div>

          <!-- Actions -->
          <div class="flex gap-2">
            <button class="flex-1 rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-stone-800 text-stone-200 hover:bg-stone-700' : 'bg-stone-200 text-stone-800 hover:bg-stone-300'"
              @click="showExportDialog = false">{{ t('share.close') }}</button>
            <button class="flex-1 rounded-xl px-4 py-2 text-sm font-medium transition"
              :class="isDark ? 'bg-emerald-600 text-white hover:bg-emerald-500' : 'bg-emerald-600 text-white hover:bg-emerald-700'"
              :disabled="exporting"
              @click="doExport">
              {{ exporting ? t('detail.exporting') : t('detail.exportBtn') }}
            </button>
          </div>
        </div>
      </div>
    </teleport>

    <!-- ═══════════════════════════════════════════
         Add to Album Modal
         ═══════════════════════════════════════════ -->
    <teleport to="body">
      <div v-if="showAddToAlbumModal" class="fixed inset-0 z-[110] flex items-center justify-center bg-black/50 backdrop-blur-sm" @click.self="showAddToAlbumModal = false">
        <div class="flex max-h-[80vh] w-full max-w-md flex-col rounded-2xl border shadow-2xl"
          :class="isDark ? 'border-stone-700 bg-stone-900 text-stone-100' : 'border-stone-200 bg-white text-stone-900'">
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
      </div>
    </teleport>
  </section>
</template>

<style scoped>
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

  .drawer-backdrop-enter-active,
  .drawer-backdrop-leave-active {
    transition: opacity 180ms ease;
  }

  .drawer-backdrop-enter-from,
  .drawer-backdrop-leave-to {
    opacity: 0;
  }

  .mobile-drawer-enter-active,
  .mobile-drawer-leave-active {
    transition: transform 220ms ease-out;
  }

  .mobile-drawer-enter-from,
  .mobile-drawer-leave-to {
    transform: translateX(-100%);
  }

  .mobile-drawer-right-enter-active,
  .mobile-drawer-right-leave-active {
    transition: transform 220ms ease-out;
  }

  .mobile-drawer-right-enter-from,
  .mobile-drawer-right-leave-to {
    transform: translateX(100%);
  }
}

@media (prefers-reduced-motion: reduce) {
  .album-view-enter-active,
  .album-view-leave-active,
  .drawer-backdrop-enter-active,
  .drawer-backdrop-leave-active,
  .mobile-drawer-enter-active,
  .mobile-drawer-leave-active,
  .mobile-drawer-right-enter-active,
  .mobile-drawer-right-leave-active {
    transition: none;
  }
}
</style>
