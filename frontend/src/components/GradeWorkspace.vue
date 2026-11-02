<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { IconBulb, IconChevronLeft, IconChevronRight, IconGrade, IconMenu, IconRefresh } from "../utils/icons";
import { t } from "../i18n/index.js";
import { httpFetch } from "../utils/http";
import { formatDurationMs, isVideoPhoto } from "../utils/mediaKind.js";
import PhotoMetaPanel from "./PhotoMetaPanel.vue";
import GradeFilmstrip from "./grade/GradeFilmstrip.vue";
import GradeHistogram from "./grade/GradeHistogram.vue";
import { nodeView } from "./grade/node_view/views.js";

const GROUPS = {
  base: ["input", "white-balance", "demosaic", "input-color-profile", "rotate", "crop", "export"],
  tone: ["exposure", "local-contrast", "sigmoid"],
  color: ["velvia", "color-zones", "lut"],
  correct: ["dehaze", "denoise", "sharpen"],
  effect: [],
};

const GROUP_ORDER = ["active", "base", "tone", "color", "correct", "effect"];

const props = defineProps({
  photo: { type: Object, default: null },
  photos: { type: Array, default: () => [] },
  isDark: { type: Boolean, default: false },
  formatSize: { type: Function, required: true },
  availableTags: { type: Array, default: () => [] },
  thumbFor: { type: Function, required: true },
});

const emit = defineEmits([
  "select",
  "download",
  "export",
  "save-meta",
  "load-more",
]);

const nodes = ref([]);
const preview = ref("");
const view = ref({ degrees: 0, roi: { x: 0, y: 0, w: 1, h: 1 } });
const rev = ref(0);
const loading = ref(false);
const fullReady = ref(false);
const assessment = ref(false);
const error = ref("");
const group = ref("active");
const activeIds = ref([]);
const expandedId = ref("");
const menuId = ref("");
const infoOpen = ref(true);
const editRating = ref(0);
const editColorLabels = ref([]);
const editTags = ref([]);
const STRIP_DEFAULT = 96;
const STRIP_MIN = 72;

const stripRef = ref(null);
const rootRef = ref(null);
const stageRef = ref(null);
const stripHeight = ref(STRIP_DEFAULT);
let stripDrag = null;
const previewImgRef = ref(null);
const infoBodyRef = ref(null);
const infoWidth = ref(0);
const viewScale = ref(1);
const viewX = ref(0);
const viewY = ref(0);
const panning = ref(false);
const navDragging = ref(false);
const stageSize = ref({ w: 0, h: 0 });
const naturalSize = ref({ w: 0, h: 0 });
let panOrigin = null;
let navDrag = null;
let viewObserver = null;

let openAbort = null;
let patchAbort = null;
let refineAbort = null;
let previewAbort = null;
let quietTimer = 0;
let patchTimer = 0;
let patchSeq = 0;
let refineSeq = 0;
let previewSeq = 0;
let patchInFlight = false;
let openInFlight = false;
let wantFull = false;
let sessionRecoveries = 0;
const pendingPatches = new Map();
const PATCH_QUIET_MS = 200;
const PHOTO_QUIET_MS = 80;

function colorLabels(value) {
  const labels = Array.isArray(value) ? value : String(value || "").split(",");
  return labels.map((label) => String(label).trim()).filter(Boolean);
}

const dimensions = computed(() => {
  const photo = props.photo;
  if (!photo || !(photo.width > 0) || !(photo.height > 0)) return "";
  return `${photo.width} * ${photo.height}`;
});
const durationLabel = computed(() => formatDurationMs(props.photo?.duration_ms));
const caption = computed(() => {
  const photo = props.photo;
  if (!photo) return "";
  const parts = [];
  if (photo.iso) parts.push(`ISO ${photo.iso}`);
  if (photo.focal_length) parts.push(String(photo.focal_length));
  if (photo.aperture) parts.push(String(photo.aperture));
  if (photo.shutter_speed) parts.push(String(photo.shutter_speed));
  return parts.join(" · ");
});
const shown = computed(() => {
  if (group.value === "active") {
    const ids = activeIds.value;
    return nodes.value.filter((node) => ids.includes(node.id));
  }
  const ids = GROUPS[group.value] || [];
  return ids.map((id) => nodes.value.find((node) => node.id === id)).filter(Boolean);
});
watch(() => props.photo?.path, async (path) => {
  const photo = props.photo;
  editRating.value = photo?.rating || 0;
  editColorLabels.value = colorLabels(photo?.color_label);
  editTags.value = (photo?.tags || []).map((tag) => String(tag).trim()).filter(Boolean);
  nodes.value = [];
  preview.value = "";
  view.value = { degrees: 0, roi: { x: 0, y: 0, w: 1, h: 1 } };
  error.value = "";
  naturalSize.value = { w: 0, h: 0 };
  resetView();
  activeIds.value = [];
  expandedId.value = "";
  menuId.value = "";
  if (!path) {
    rev.value = 0;
    return;
  }
  void openSession();
  await nextTick();
  stripRef.value?.scrollCurrentIntoView();
}, { immediate: true });

watch(nodes, (list) => {
  if (group.value === "active" && activeIds.value.length === 0 && list.length) {
    activeIds.value = list.filter((node) => node.enabled).map((node) => node.id);
  }
});

onMounted(() => {
  bindViewObserver();
  window.addEventListener("keydown", onPhotoKeydown);
});
watch(previewImgRef, () => bindViewObserver());
watch(infoBodyRef, () => bindViewObserver());

onBeforeUnmount(() => {
  window.removeEventListener("keydown", onPhotoKeydown);
  viewObserver?.disconnect();
  openAbort?.abort();
  previewAbort?.abort();
  patchAbort?.abort();
  patchAbort = null;
  refineAbort?.abort();
  window.clearTimeout(quietTimer);
  window.clearTimeout(patchTimer);
  pendingPatches.clear();
  wantFull = false;
  endStripDrag();
});

function groupTitle(id) {
  return t(`grade.group.${id}`);
}

function syncMeta(photo) {
  if (!photo) return;
  editRating.value = photo.rating || 0;
  editColorLabels.value = colorLabels(photo.color_label);
  editTags.value = (photo.tags || []).map((tag) => String(tag).trim()).filter(Boolean);
}

async function openSession() {
  const photo = props.photo;
  if (!photo?.path) return;
  openAbort?.abort();
  previewAbort?.abort();
  patchAbort?.abort();
  patchAbort = null;
  pendingPatches.clear();
  window.clearTimeout(patchTimer);
  patchSeq += 1;
  refineSeq += 1;
  armFull(PHOTO_QUIET_MS);
  const ctrl = new AbortController();
  openAbort = ctrl;
  openInFlight = true;
  loading.value = true;
  error.value = "";
  let opened = false;
  try {
    const body = { path: photo.path };
    if (photo.owner) body.owner = photo.owner;
    const res = await httpFetch("/api/grade/open", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      signal: ctrl.signal,
      body: JSON.stringify(body),
    });
    const data = await res.json().catch(() => ({}));
    if (ctrl.signal.aborted || props.photo?.path !== photo.path) return;
    if (!res.ok) {
      error.value = data.message || t("grade.failed");
      if (openAbort === ctrl) {
        window.clearTimeout(quietTimer);
        wantFull = false;
      }
      return;
    }
    rev.value = data.rev;
    nodes.value = data.nodes || [];
    fullReady.value = false;
    opened = true;
    void requestPreview(data.rev);
  } catch (err) {
    if (err?.name === "AbortError") return;
    if (props.photo?.path === photo.path) {
      error.value = err.message || t("grade.failed");
      window.clearTimeout(quietTimer);
      wantFull = false;
    }
  } finally {
    if (openAbort === ctrl) {
      loading.value = false;
      openInFlight = false;
    }
  }
  if (opened && openAbort === ctrl) maybeRequestFull();
}

async function requestPreview(revAtStart) {
  previewAbort?.abort();
  const seq = ++previewSeq;
  const ctrl = new AbortController();
  previewAbort = ctrl;
  try {
    const res = await httpFetch("/api/grade/preview", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      signal: ctrl.signal,
      body: JSON.stringify({ rev: revAtStart }),
    });
    const data = await res.json().catch(() => ({}));
    if (ctrl.signal.aborted || seq !== previewSeq || rev.value !== revAtStart) return;
    if (!res.ok || !data.preview || data.superseded || fullReady.value) return;
    preview.value = data.preview;
    applyView(data);
  } catch (err) {
    if (err?.name === "AbortError") return;
  }
}

function armFull(ms) {
  refineAbort?.abort();
  wantFull = false;
  window.clearTimeout(quietTimer);
  quietTimer = window.setTimeout(() => {
    wantFull = true;
    maybeRequestFull();
  }, ms);
}

function schedulePatch(node) {
  pendingPatches.set(node.id, node);
  patchSeq += 1;
  patchAbort?.abort();
  refineAbort?.abort();
  wantFull = false;
  window.clearTimeout(quietTimer);
  window.clearTimeout(patchTimer);
  patchTimer = window.setTimeout(() => {
    void flushPatches();
  }, PATCH_QUIET_MS);
}

function maybeRequestFull() {
  if (!wantFull || patchInFlight || openInFlight || pendingPatches.size || !rev.value) return;
  wantFull = false;
  void requestFull(rev.value);
}

function applyView(data) {
  const next = data?.view;
  const roi = next?.roi;
  if (!roi) return;
  view.value = {
    degrees: Number(next.degrees) || 0,
    roi: {
      x: Number(roi.x) || 0,
      y: Number(roi.y) || 0,
      w: Number(roi.w) || 1,
      h: Number(roi.h) || 1,
    },
  };
}

function sessionGone(data) {
  const reason = data?.data?.reason;
  return reason === "not-open" || reason === "stale";
}

function restoreUnsent(nodes) {
  for (const node of nodes) {
    if (!pendingPatches.has(node.id)) pendingPatches.set(node.id, node);
  }
}

async function flushPatches() {
  if (openInFlight) {
    window.clearTimeout(patchTimer);
    patchTimer = window.setTimeout(() => {
      void flushPatches();
    }, PATCH_QUIET_MS);
    return;
  }
  if (pendingPatches.size === 0) return;
  const seq = patchSeq;
  const batch = [...pendingPatches.values()];
  pendingPatches.clear();
  const ctrl = new AbortController();
  patchAbort = ctrl;
  patchInFlight = true;
  loading.value = true;
  let finished = false;
  let reopenFrom = -1;
  try {
    for (let i = 0; i < batch.length; i += 1) {
      if (ctrl.signal.aborted || seq !== patchSeq) {
        restoreUnsent(batch.slice(i));
        return;
      }
      const status = await postNode(batch[i], ctrl.signal, seq);
      if (status === "reopen") {
        reopenFrom = i;
        break;
      }
      if (status === "superseded") return;
      if (status !== "ok") {
        if (status === "stale") restoreUnsent(batch.slice(i));
        return;
      }
    }
    finished = reopenFrom < 0;
  } finally {
    if (patchAbort === ctrl) {
      patchInFlight = false;
      if (pendingPatches.size === 0) loading.value = false;
    }
  }
  if (reopenFrom >= 0) {
    if (sessionRecoveries >= 1) {
      error.value = t("grade.failed");
      return;
    }
    sessionRecoveries += 1;
    const photoPath = props.photo?.path;
    const revBefore = rev.value;
    const leftover = batch.slice(reopenFrom);
    await openSession();
    if (props.photo?.path !== photoPath || !rev.value || rev.value === revBefore) return;
    restoreUnsent(leftover);
    void flushPatches();
    return;
  }
  if (finished && patchAbort === ctrl && seq === patchSeq && pendingPatches.size === 0) {
    armFull(0);
  }
}

async function postNode(node, signal, seq) {
  const photo = props.photo;
  const photoPath = photo?.path || "";
  const sentRev = rev.value;
  if (!photoPath || !sentRev || seq !== patchSeq || signal.aborted) return "stale";
  try {
    const res = await httpFetch("/api/grade/node", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      signal,
      body: JSON.stringify({
        id: node.id,
        enabled: node.enabled,
        params: JSON.parse(JSON.stringify(node.params)),
        rev: sentRev,
      }),
    });
    const data = await res.json().catch(() => ({}));
    if (signal.aborted || seq !== patchSeq || rev.value !== sentRev || props.photo?.path !== photoPath) return "stale";
    if (data.superseded) return "superseded";
    if (!res.ok) {
      if (sessionGone(data)) return "reopen";
      error.value = data.message || t("grade.failed");
      return "error";
    }
    const index = nodes.value.findIndex((item) => item.id === node.id);
    if (index >= 0 && data.node && !pendingPatches.has(node.id)) {
      nodes.value.splice(index, 1, data.node);
    }
    if (data.preview) {
      preview.value = data.preview;
      fullReady.value = false;
      applyView(data);
    }
    sessionRecoveries = 0;
    return "ok";
  } catch (err) {
    if (err?.name === "AbortError" || signal.aborted || seq !== patchSeq) return "stale";
    error.value = err.message || t("grade.failed");
    return "error";
  }
}

async function requestFull(revAtStart) {
  refineAbort?.abort();
  const seq = ++refineSeq;
  const ctrl = new AbortController();
  refineAbort = ctrl;
  try {
    const res = await httpFetch("/api/grade/full", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      signal: ctrl.signal,
      body: JSON.stringify({ rev: revAtStart }),
    });
    const data = await res.json().catch(() => ({}));
    if (ctrl.signal.aborted || seq !== refineSeq || rev.value !== revAtStart) return;
    if (!res.ok || !data.preview) return;
    preview.value = data.preview;
    fullReady.value = true;
    applyView(data);
  } catch (err) {
    if (err?.name === "AbortError") return;
  }
}

function selectGroup(id) {
  if (group.value === id) return;
  group.value = id;
  expandedId.value = "";
  menuId.value = "";
  if (id === "active") {
    activeIds.value = nodes.value.filter((node) => node.enabled).map((node) => node.id);
  }
}

function toggleExpand(id) {
  menuId.value = "";
  expandedId.value = expandedId.value === id ? "" : id;
}

function toggleEnabled(node) {
  if (node.always_on) return;
  node.enabled = !node.enabled;
  schedulePatch(node);
}

function markEdited(node) {
  node.enabled = true;
  schedulePatch(node);
}

function resetNode(node) {
  if (!node.defaults) return;
  const defaults = JSON.parse(JSON.stringify(node.defaults));
  node.params = defaults;
  for (const field of node.fields || []) {
    const value = defaults[field.key];
    if (value === undefined) continue;
    if (field.kind === "slider") {
      field.value = Number(value);
    } else if (field.kind === "toggle") {
      field.value = value ? 1 : 0;
    } else if (field.kind === "choice") {
      const index = (field.options || []).findIndex((option) => option.value === value);
      if (index >= 0) {
        field.value = index;
        field.display = field.options[index].label;
      }
    }
  }
  markEdited(node);
}

function toggleMenu(id) {
  menuId.value = menuId.value === id ? "" : id;
}

function onSlider(node, field, value) {
  const next = Number(value);
  field.value = next;
  node.params[field.key] = next;
  markEdited(node);
}

function onChoice(node, field, option, index) {
  field.value = index;
  field.display = option.label;
  node.params[field.key] = option.value;
  markEdited(node);
}

function onToggle(node, field, on) {
  field.value = on ? 1 : 0;
  node.params[field.key] = on;
  markEdited(node);
}

async function importProfile(file) {
  const sentRev = rev.value;
  const photoPath = props.photo?.path || "";
  if (!sentRev || !file || !photoPath) return;
  const profile = nodes.value.find((node) => node.id === "input-color-profile");
  if (profile) profile.enabled = true;
  patchSeq += 1;
  const seq = patchSeq;
  patchAbort?.abort();
  refineAbort?.abort();
  wantFull = false;
  window.clearTimeout(quietTimer);
  window.clearTimeout(patchTimer);
  const ctrl = new AbortController();
  patchAbort = ctrl;
  patchInFlight = true;
  const body = new FormData();
  body.append("rev", String(sentRev));
  body.append("file", file, file.name);
  loading.value = true;
  error.value = "";
  try {
    const res = await httpFetch("/api/grade/profile", {
      method: "POST",
      signal: ctrl.signal,
      body,
    });
    const data = await res.json().catch(() => ({}));
    if (ctrl.signal.aborted || seq !== patchSeq || rev.value !== sentRev || props.photo?.path !== photoPath) return;
    if (data.superseded) return;
    if (!res.ok) {
      if (sessionGone(data) && props.photo?.path === photoPath) {
        await openSession();
        return;
      }
      error.value = data.message || t("grade.failed");
      return;
    }
    const index = nodes.value.findIndex((item) => item.id === "input-color-profile");
    if (index >= 0 && data.node) nodes.value.splice(index, 1, data.node);
    if (data.preview) {
      preview.value = data.preview;
      fullReady.value = false;
      applyView(data);
    }
    armFull(PATCH_QUIET_MS);
  } catch (err) {
    if (err?.name === "AbortError" || ctrl.signal.aborted) return;
    if (seq === patchSeq) error.value = err.message || t("grade.failed");
  } finally {
    if (patchAbort === ctrl) {
      patchInFlight = false;
      if (pendingPatches.size === 0) loading.value = false;
    }
  }
}

function pickPhoto(photo) {
  if (!photo || isVideoPhoto(photo)) return;
  if (photo.path === props.photo?.path) return;
  emit("select", photo);
}

function stepPhoto(direction) {
  const list = props.photos || [];
  const current = props.photo?.path;
  let index = list.findIndex((item) => item?.path === current);
  if (index < 0) index = direction > 0 ? -1 : list.length;
  for (let i = index + direction; i >= 0 && i < list.length; i += direction) {
    const item = list[i];
    if (!item?.path || isVideoPhoto(item)) continue;
    pickPhoto(item);
    return;
  }
}

function onPhotoKeydown(event) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
  const target = event.target;
  if (target instanceof HTMLElement && target.closest("input, textarea, select, [contenteditable='true']")) return;
  event.preventDefault();
  stepPhoto(event.key === "ArrowLeft" ? -1 : 1);
}

function setRating(star) {
  editRating.value = editRating.value === star ? 0 : star;
  saveMeta();
}

function toggleColor(key) {
  const index = editColorLabels.value.indexOf(key);
  if (index >= 0) editColorLabels.value.splice(index, 1);
  else editColorLabels.value.push(key);
  saveMeta();
}

function onTags(tags) {
  editTags.value = tags;
  saveMeta();
}

function saveMeta() {
  const photo = props.photo;
  if (!photo) return;
  const tags = editTags.value.map((tag) => String(tag).trim()).filter(Boolean);
  emit("save-meta", {
    path: photo.path,
    album: photo.album,
    tags,
    rating: editRating.value,
    colorLabel: [...editColorLabels.value],
  });
  syncMeta({
    ...photo,
    rating: editRating.value,
    color_label: [...editColorLabels.value],
    tags,
  });
}

const placed = computed(() => {
  const imageW = naturalSize.value.w;
  const imageH = naturalSize.value.h;
  if (!imageW || !imageH) return null;
  const rad = (Number(view.value.degrees) || 0) * Math.PI / 180;
  const c = Math.abs(Math.cos(rad));
  const s = Math.abs(Math.sin(rad));
  const canvasW = imageW * c + imageH * s;
  const canvasH = imageW * s + imageH * c;
  const roi = view.value.roi || { x: 0, y: 0, w: 1, h: 1 };
  const showCanvas = expandedId.value === "rotate";
  const frameW = showCanvas ? canvasW : Math.max(canvasW * roi.w, 1);
  const frameH = showCanvas ? canvasH : Math.max(canvasH * roi.h, 1);
  return { imageW, imageH, canvasW, canvasH, roi, showCanvas, frameW, frameH };
});

const matPx = computed(() => {
  if (!assessment.value) return 0;
  const stageW = stageSize.value.w;
  const stageH = stageSize.value.h;
  if (!stageW || !stageH) return 0;
  return Math.max(36, Math.round(Math.min(stageW, stageH) * 0.08));
});

const fitSize = computed(() => {
  const stageW = stageSize.value.w;
  const stageH = stageSize.value.h;
  const place = placed.value;
  if (!stageW || !stageH || !place) return null;
  const mat = matPx.value;
  const availW = Math.max(1, stageW - mat * 2);
  const availH = Math.max(1, stageH - mat * 2);
  const imageAspect = place.frameW / place.frameH;
  const stageAspect = availW / availH;
  if (imageAspect >= stageAspect) {
    const w = availW;
    return { w, h: w / imageAspect };
  }
  const h = availH;
  return { w: h * imageAspect, h };
});

const canvasStyle = computed(() => {
  const place = placed.value;
  if (!place) return {};
  if (place.showCanvas) return { left: "0", top: "0", width: "100%", height: "100%" };
  const { roi } = place;
  return {
    left: `${(-roi.x / roi.w) * 100}%`,
    top: `${(-roi.y / roi.h) * 100}%`,
    width: `${(1 / roi.w) * 100}%`,
    height: `${(1 / roi.h) * 100}%`,
  };
});

const bitmapStyle = computed(() => {
  const place = placed.value;
  if (!place) return {};
  const w = (place.imageW / place.canvasW) * 100;
  const h = (place.imageH / place.canvasH) * 100;
  return {
    left: `${(100 - w) / 2}%`,
    top: `${(100 - h) / 2}%`,
    width: `${w}%`,
    height: `${h}%`,
    maxWidth: "none",
    transform: `rotate(${view.value.degrees || 0}deg)`,
    transformOrigin: "center center",
  };
});

const viewMetrics = computed(() => {
  const fit = fitSize.value;
  const stageW = stageSize.value.w;
  const stageH = stageSize.value.h;
  if (!fit || !stageW || !stageH) return null;
  const scale = viewScale.value;
  const imageW = fit.w * scale;
  const imageH = fit.h * scale;
  const mat = matPx.value;
  if (mat <= 0) {
    return { imageW, imageH, clipW: stageW, clipH: stageH, whiteW: 0, whiteH: 0, mat: 0 };
  }
  const whiteW = Math.min(stageW, imageW + mat * 2);
  const whiteH = Math.min(stageH, imageH + mat * 2);
  return {
    imageW,
    imageH,
    clipW: Math.max(1, whiteW - mat * 2),
    clipH: Math.max(1, whiteH - mat * 2),
    whiteW,
    whiteH,
    mat,
  };
});

const whiteStyle = computed(() => {
  const metrics = viewMetrics.value;
  const stageW = stageSize.value.w;
  const stageH = stageSize.value.h;
  if (!metrics || metrics.mat <= 0) return { left: "0", top: "0", width: "100%", height: "100%" };
  return {
    left: `${(stageW - metrics.whiteW) / 2}px`,
    top: `${(stageH - metrics.whiteH) / 2}px`,
    width: `${metrics.whiteW}px`,
    height: `${metrics.whiteH}px`,
    background: "#ffffff",
  };
});

const clipStyle = computed(() => {
  const metrics = viewMetrics.value;
  if (!metrics || metrics.mat <= 0) return { left: "0", top: "0", width: "100%", height: "100%" };
  return {
    left: `${metrics.mat}px`,
    top: `${metrics.mat}px`,
    width: `${metrics.clipW}px`,
    height: `${metrics.clipH}px`,
  };
});

const imageStyle = computed(() => {
  const fit = fitSize.value;
  const metrics = viewMetrics.value;
  if (!fit || !metrics) return {};
  if (metrics.mat <= 0) {
    return {
      width: `${fit.w}px`,
      height: `${fit.h}px`,
      left: "50%",
      top: "50%",
      transform: `translate(-50%, -50%) translate(${viewX.value}px, ${viewY.value}px) scale(${viewScale.value})`,
      transformOrigin: "center center",
    };
  }
  return {
    width: `${metrics.imageW}px`,
    height: `${metrics.imageH}px`,
    left: "50%",
    top: "50%",
    transform: `translate(calc(-50% + ${viewX.value}px), calc(-50% + ${viewY.value}px))`,
  };
});

watch(fitSize, (next, prev) => {
  if (!next || !prev || !prev.w || !prev.h) return;
  if (next.w === prev.w && next.h === prev.h) return;
  viewX.value *= next.w / prev.w;
  viewY.value *= next.h / prev.h;
  clampView();
});

function measureView() {
  const stage = stageRef.value;
  const nextStage = { w: stage?.clientWidth || 0, h: stage?.clientHeight || 0 };
  if (nextStage.w !== stageSize.value.w || nextStage.h !== stageSize.value.h) stageSize.value = nextStage;
  const body = infoBodyRef.value;
  if (!body) return;
  const style = getComputedStyle(body);
  const pad = (parseFloat(style.paddingLeft) || 0) + (parseFloat(style.paddingRight) || 0);
  const width = Math.max(0, body.clientWidth - pad);
  if (width !== infoWidth.value) infoWidth.value = width;
}

function onPreviewLoad(event) {
  const img = event.target;
  const w = img?.naturalWidth || 0;
  const h = img?.naturalHeight || 0;
  if (w > 0 && h > 0 && (w !== naturalSize.value.w || h !== naturalSize.value.h)) {
    naturalSize.value = { w, h };
  }
  measureView();
}

function bindViewObserver() {
  if (!viewObserver) viewObserver = new ResizeObserver(() => measureView());
  viewObserver.disconnect();
  if (stageRef.value) viewObserver.observe(stageRef.value);
  if (previewImgRef.value) viewObserver.observe(previewImgRef.value);
  if (infoBodyRef.value) viewObserver.observe(infoBodyRef.value);
  measureView();
}

function thumbSize() {
  const imgW = fitSize.value?.w || 0;
  const imgH = fitSize.value?.h || 0;
  if (!imgW || !imgH) return null;
  const maxW = infoWidth.value || 240;
  const maxH = 160;
  const aspect = imgW / imgH;
  let w = maxW;
  let h = w / aspect;
  if (h > maxH) {
    h = maxH;
    w = h * aspect;
  }
  return { w, h };
}

const navBoxStyle = computed(() => {
  const thumb = thumbSize();
  if (!thumb) return { width: "100%", maxHeight: "10rem" };
  return { width: `${thumb.w}px`, height: `${thumb.h}px` };
});

const navigatorBox = computed(() => {
  const thumb = thumbSize();
  const metrics = viewMetrics.value;
  if (!thumb || !metrics || viewScale.value <= 1) return null;
  const imgW = fitSize.value?.w || 0;
  const imgH = fitSize.value?.h || 0;
  if (!imgW || !imgH) return null;
  if (metrics.imageW <= metrics.clipW + 1 && metrics.imageH <= metrics.clipH + 1) return null;
  const left = metrics.clipW / 2 + viewX.value - metrics.imageW / 2;
  const top = metrics.clipH / 2 + viewY.value - metrics.imageH / 2;
  const ix0 = Math.max(0, Math.min(imgW, -left / viewScale.value));
  const iy0 = Math.max(0, Math.min(imgH, -top / viewScale.value));
  const ix1 = Math.max(ix0, Math.min(imgW, (metrics.clipW - left) / viewScale.value));
  const iy1 = Math.max(iy0, Math.min(imgH, (metrics.clipH - top) / viewScale.value));
  return {
    thumbW: thumb.w,
    thumbH: thumb.h,
    x: (ix0 / imgW) * thumb.w,
    y: (iy0 / imgH) * thumb.h,
    w: Math.max(1, ((ix1 - ix0) / imgW) * thumb.w),
    h: Math.max(1, ((iy1 - iy0) / imgH) * thumb.h),
  };
});

function panToThumbPoint(tx, ty) {
  const thumb = thumbSize();
  const imgW = fitSize.value?.w || 0;
  const imgH = fitSize.value?.h || 0;
  const metrics = viewMetrics.value;
  const scale = viewScale.value;
  if (!thumb || !navDrag || !imgW || !imgH || !metrics) return;
  const ix0 = ((tx - navDrag.ox) / thumb.w) * imgW;
  const iy0 = ((ty - navDrag.oy) / thumb.h) * imgH;
  viewX.value = (imgW / 2 - ix0) * scale - metrics.clipW / 2;
  viewY.value = (imgH / 2 - iy0) * scale - metrics.clipH / 2;
  clampView();
}

function onNavPointerDown(event) {
  if (event.button !== 0) return;
  measureView();
  const box = navigatorBox.value;
  if (!box) return;
  const rect = event.currentTarget.getBoundingClientRect();
  const tx = event.clientX - rect.left;
  const ty = event.clientY - rect.top;
  const pad = 6;
  const inside = tx >= box.x - pad && tx <= box.x + box.w + pad && ty >= box.y - pad && ty <= box.y + box.h + pad;
  navDrag = {
    ox: inside ? tx - box.x : box.w / 2,
    oy: inside ? ty - box.y : box.h / 2,
  };
  navDragging.value = true;
  panToThumbPoint(tx, ty);
  event.currentTarget.setPointerCapture(event.pointerId);
  event.preventDefault();
}

function onNavPointerMove(event) {
  if (!navDrag) return;
  const rect = event.currentTarget.getBoundingClientRect();
  panToThumbPoint(event.clientX - rect.left, event.clientY - rect.top);
}

function onNavPointerUp() {
  navDrag = null;
  navDragging.value = false;
}

function resetView() {
  viewScale.value = 1;
  viewX.value = 0;
  viewY.value = 0;
  panning.value = false;
  panOrigin = null;
}

function onStageWheel(event) {
  const stage = stageRef.value;
  if (!stage || !preview.value) return;
  measureView();
  const rect = stage.getBoundingClientRect();
  const cx = event.clientX - rect.left - rect.width / 2;
  const cy = event.clientY - rect.top - rect.height / 2;
  const factor = event.deltaY < 0 ? 1.12 : 1 / 1.12;
  const prev = viewScale.value;
  const next = Math.min(8, Math.max(1, prev * factor));
  if (next === prev) return;
  const ratio = next / prev;
  viewX.value = cx - (cx - viewX.value) * ratio;
  viewY.value = cy - (cy - viewY.value) * ratio;
  viewScale.value = next;
  clampView();
}

function clampView() {
  if (viewScale.value <= 1) {
    viewScale.value = 1;
    viewX.value = 0;
    viewY.value = 0;
    return;
  }
  const metrics = viewMetrics.value;
  if (!metrics) return;
  const maxX = Math.max(0, (metrics.imageW - metrics.clipW) / 2);
  const maxY = Math.max(0, (metrics.imageH - metrics.clipH) / 2);
  viewX.value = Math.min(maxX, Math.max(-maxX, viewX.value));
  viewY.value = Math.min(maxY, Math.max(-maxY, viewY.value));
}

function onStagePointerDown(event) {
  if (event.button !== 0 || viewScale.value <= 1) return;
  panning.value = true;
  panOrigin = {
    x: event.clientX,
    y: event.clientY,
    vx: viewX.value,
    vy: viewY.value,
  };
  event.currentTarget.setPointerCapture(event.pointerId);
}

function onStagePointerMove(event) {
  if (!panning.value || !panOrigin) return;
  viewX.value = panOrigin.vx + event.clientX - panOrigin.x;
  viewY.value = panOrigin.vy + event.clientY - panOrigin.y;
  clampView();
}

function onStagePointerUp() {
  panning.value = false;
  panOrigin = null;
}

function stripMax() {
  const root = rootRef.value;
  if (!root) return 360;
  return Math.max(STRIP_MIN, root.clientHeight - 160);
}

function onStripPointerDown(event) {
  if (event.button !== 0) return;
  stripDrag = { y: event.clientY, h: stripHeight.value };
  document.body.style.cursor = "row-resize";
  event.currentTarget.setPointerCapture(event.pointerId);
}

function onStripPointerMove(event) {
  if (!stripDrag) return;
  const next = stripDrag.h + (stripDrag.y - event.clientY);
  stripHeight.value = Math.round(Math.min(stripMax(), Math.max(STRIP_MIN, next)));
}

function endStripDrag() {
  if (!stripDrag && document.body.style.cursor !== "row-resize") return;
  stripDrag = null;
  if (document.body.style.cursor === "row-resize") document.body.style.cursor = "";
}

defineExpose({ syncMeta });
</script>

<template>
  <div ref="rootRef" class="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden" @click="menuId = ''">
    <div class="flex min-h-0 flex-1">
      <aside class="app-split-r flex shrink-0 flex-col overflow-hidden"
        :class="infoOpen ? 'w-72' : 'w-10'">
        <button v-if="!infoOpen" type="button"
          class="flex h-full w-full flex-col items-center gap-2 py-3"
          :class="isDark ? 'text-stone-400 hover:bg-white/5' : 'text-stone-500 hover:bg-white/50'"
          :title="t('grade.showInfo')"
          @click="infoOpen = true">
          <IconChevronRight class="h-3.5 w-3.5 shrink-0" />
          <span v-if="photo" class="max-h-48 truncate text-[11px] [writing-mode:vertical-rl]">{{ photo.name }}</span>
        </button>
        <template v-else>
          <button type="button" class="flex shrink-0 items-center gap-1 px-3 pt-3 text-[11px]"
            :class="isDark ? 'text-stone-400 hover:text-stone-200' : 'text-stone-500 hover:text-stone-800'"
            @click="infoOpen = false">
            <IconChevronLeft class="h-3.5 w-3.5" />
            {{ t('grade.hideInfo') }}
          </button>
          <div ref="infoBodyRef" class="app-scrollbar min-h-0 flex-1 overflow-y-auto px-3 py-3">
        <div v-if="photo" class="mb-2 flex w-full justify-center">
        <div
          class="relative overflow-hidden rounded"
          :class="navigatorBox ? ['touch-none', navDragging ? 'cursor-grabbing' : 'cursor-grab'] : ''"
          :style="navBoxStyle"
          :title="navigatorBox ? t('grade.navigator') : ''"
          @pointerdown.stop="onNavPointerDown"
          @pointermove="onNavPointerMove"
          @pointerup="onNavPointerUp"
          @pointercancel="onNavPointerUp">
          <div v-if="preview" class="absolute inset-0 overflow-hidden">
            <div class="absolute" :style="canvasStyle">
              <img :src="preview" alt="" draggable="false"
                class="pointer-events-none absolute select-none object-fill"
                :style="bitmapStyle" />
            </div>
          </div>
          <img v-else :src="thumbFor(photo)" alt="" draggable="false"
            class="pointer-events-none block max-h-40 w-full select-none object-contain" />
          <div v-if="navigatorBox"
            class="pointer-events-none absolute shadow-[inset_0_0_0_2px_white,0_0_0_999px_rgba(0,0,0,0.5)]"
            :style="{ left: `${navigatorBox.x}px`, top: `${navigatorBox.y}px`, width: `${navigatorBox.w}px`, height: `${navigatorBox.h}px` }"></div>
        </div>
        </div>
        <PhotoMetaPanel
          v-if="photo"
          :photo="photo"
          :is-dark="isDark"
          :format-size="formatSize"
          :dimensions="dimensions"
          :duration-label="durationLabel"
          :rating="editRating"
          :color-label-keys="editColorLabels"
          :tags="editTags"
          :available-tags="availableTags"
          :is-video="false"
          :actions="false"
          @download="emit('download', photo)"
          @export="emit('export', photo)"
          @set-rating="setRating"
          @toggle-color="toggleColor"
          @update:tags="onTags"
        />
          </div>
        </template>
      </aside>

      <div ref="stageRef"
        class="relative flex min-h-0 min-w-0 flex-1 items-center justify-center overflow-hidden"
        :class="viewScale > 1 ? (panning ? 'cursor-grabbing' : 'cursor-grab') : ''"
        @wheel.prevent="onStageWheel"
        @pointerdown="onStagePointerDown"
        @pointermove="onStagePointerMove"
        @pointerup="onStagePointerUp"
        @pointercancel="onStagePointerUp"
        @dblclick="resetView">
        <div v-if="preview" class="absolute" :style="whiteStyle">
          <div class="absolute overflow-hidden" :style="clipStyle">
            <div class="absolute" :class="placed?.showCanvas ? '' : 'overflow-hidden'" :style="imageStyle">
              <div class="absolute" :style="canvasStyle">
                <img ref="previewImgRef" :src="preview" alt="" draggable="false"
                  class="absolute select-none object-fill"
                  :style="bitmapStyle"
                  @load="onPreviewLoad" />
              </div>
              <div v-if="placed?.showCanvas" class="pointer-events-none absolute inset-0 overflow-hidden">
                <div class="absolute border-2 border-white shadow-[0_0_0_999px_rgba(0,0,0,0.55)]"
                  :style="{
                    left: `${(view.roi?.x || 0) * 100}%`,
                    top: `${(view.roi?.y || 0) * 100}%`,
                    width: `${(view.roi?.w || 1) * 100}%`,
                    height: `${(view.roi?.h || 1) * 100}%`,
                  }"></div>
              </div>
            </div>
          </div>
        </div>
        <IconGrade v-if="loading || (preview && !fullReady)"
          class="pointer-events-none absolute left-3 top-3 z-10 h-7 w-7 animate-spin"
          :title="loading ? t('grade.loading') : ''" />
        <p v-if="!preview && error" class="px-6 text-center text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-600'">{{ error }}</p>
        <p v-else-if="!preview" class="text-sm" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ t('grade.loading') }}</p>
        <p v-if="error && preview" class="absolute bottom-3 left-1/2 max-w-md -translate-x-1/2 truncate rounded bg-black/70 px-3 py-1 text-xs text-white">{{ error }}</p>
      </div>

      <aside class="flex w-80 shrink-0 flex-col border-l"
        :class="isDark ? 'border-white/12' : 'border-stone-900/15'"
        @click.stop>
        <GradeHistogram :src="preview" :caption="caption" :is-dark="isDark" />
        <div class="app-split-b flex shrink-0 gap-1 px-2 py-1.5">
          <button v-for="id in GROUP_ORDER" :key="id" type="button"
            class="flex h-9 flex-1 items-center justify-center rounded-md transition"
            :class="group === id
              ? (isDark ? 'bg-white/12 text-stone-50' : 'bg-white text-stone-900 shadow-sm')
              : (isDark ? 'text-stone-400 hover:bg-white/8 hover:text-stone-200' : 'text-stone-500 hover:bg-white/70 hover:text-stone-800')"
            :title="groupTitle(id)"
            @click="selectGroup(id)">
            <svg v-if="id === 'active'" viewBox="0 0 20 20" class="h-5 w-5" fill="currentColor" aria-hidden="true">
              <path d="M11 2a1 1 0 10-2 0v6a1 1 0 102 0V2z" />
              <path d="M6.3 3.8a1 1 0 10-1.6-1.2A7 7 0 1015.3 2.6a1 1 0 10-1.6 1.2A5 5 0 116.3 3.8z" />
            </svg>
            <svg v-else-if="id === 'base'" viewBox="0 0 20 20" class="h-5 w-5" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
              <circle cx="10" cy="10" r="5.5" />
            </svg>
            <svg v-else-if="id === 'tone'" viewBox="0 0 20 20" class="h-5 w-5" aria-hidden="true">
              <circle cx="10" cy="10" r="6" fill="currentColor" />
            </svg>
            <svg v-else-if="id === 'color'" viewBox="0 0 20 20" class="h-5 w-5" aria-hidden="true">
              <circle cx="10" cy="10" r="6" fill="url(#grade-wheel)" />
              <defs>
                <linearGradient id="grade-wheel" x1="4" y1="4" x2="16" y2="16">
                  <stop offset="0" stop-color="#f87171" />
                  <stop offset="0.5" stop-color="#4ade80" />
                  <stop offset="1" stop-color="#60a5fa" />
                </linearGradient>
              </defs>
            </svg>
            <svg v-else-if="id === 'correct'" viewBox="0 0 20 20" class="h-5 w-5" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
              <path d="M14.5 6.5A5 5 0 116.2 13" stroke-linecap="round" />
              <path d="M14.5 3.5v3h-3" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
            <svg v-else viewBox="0 0 20 20" class="h-5 w-5" fill="currentColor" aria-hidden="true">
              <path d="M10 2.5l1.2 3.6 3.8.1-3 2.3 1.1 3.6L10 10.4 6.9 12.1 8 8.5 5 6.2l3.8-.1L10 2.5z" />
            </svg>
          </button>
        </div>
        <div class="app-scrollbar min-h-0 flex-1 overflow-y-auto px-1.5 py-1">
          <p v-if="shown.length === 0" class="px-2 py-3 text-[11px]" :class="isDark ? 'text-stone-500' : 'text-stone-500'">{{ t('grade.emptyGroup') }}</p>
          <div v-for="node in shown" :key="node.id" class="relative mb-0.5">
            <div class="flex h-8 cursor-pointer select-none items-center"
              :class="expandedId === node.id
                ? (isDark ? 'bg-white/8 text-stone-100' : 'bg-white/80 text-stone-900')
                : (isDark ? 'text-stone-200 hover:bg-white/5' : 'text-stone-800 hover:bg-white/50')"
              @click="toggleExpand(node.id)">
              <span v-if="node.always_on"
                class="flex h-full w-7 shrink-0 items-center justify-center"
                :title="t('grade.alwaysOn')">
                <svg viewBox="0 0 20 20" class="h-3.5 w-3.5" fill="none" stroke="currentColor" stroke-width="1.6" aria-hidden="true">
                  <circle cx="10" cy="10" r="5.25" />
                </svg>
              </span>
              <button v-else type="button" class="flex h-full w-7 shrink-0 items-center justify-center"
                :class="node.enabled ? (isDark ? 'text-stone-100' : 'text-stone-800') : (isDark ? 'text-stone-600' : 'text-stone-400')"
                :title="node.enabled ? t('grade.disable') : t('grade.enable')"
                @click.stop="toggleEnabled(node)">
                <svg viewBox="0 0 20 20" class="h-3.5 w-3.5" aria-hidden="true">
                  <path v-if="node.enabled" fill="currentColor" d="M11 2a1 1 0 10-2 0v6a1 1 0 102 0V2z" />
                  <path v-if="node.enabled" fill="currentColor" d="M6.3 3.8a1 1 0 10-1.6-1.2A7 7 0 1015.3 2.6a1 1 0 10-1.6 1.2A5 5 0 116.3 3.8z" />
                  <g v-else fill="none" stroke="currentColor" stroke-width="1.4">
                    <path d="M11 3.2v4.2" stroke-linecap="round" />
                    <path d="M6.6 4.6a5.2 5.2 0 108.2.4" stroke-linecap="round" />
                  </g>
                </svg>
              </button>
              <span class="min-w-0 flex-1 truncate text-left text-[13px]">{{ node.title }}</span>
              <button type="button" class="flex h-full w-7 shrink-0 items-center justify-center"
                :class="isDark ? 'text-stone-500 hover:text-stone-200' : 'text-stone-400 hover:text-stone-800'"
                :title="t('grade.reset')" @click.stop="resetNode(node)">
                <IconRefresh class="h-3.5 w-3.5" />
              </button>
              <button type="button" class="flex h-full w-7 shrink-0 items-center justify-center"
                :class="isDark ? 'text-stone-500 hover:text-stone-200' : 'text-stone-400 hover:text-stone-800'"
                :title="t('grade.options')" @click.stop="toggleMenu(node.id)">
                <IconMenu class="h-3.5 w-3.5" />
              </button>
            </div>
            <div v-if="menuId === node.id" class="app-surface absolute right-1 top-8 z-10 rounded-lg border px-2 py-1.5 text-[11px] shadow-lg"
              :class="isDark ? 'border-white/10 text-stone-400' : 'border-stone-900/10 text-stone-500'">
              {{ t('grade.noPresets') }}
            </div>
            <div v-if="expandedId === node.id" class="px-2 py-1.5" :class="isDark ? 'bg-white/5' : 'bg-white/45'">
              <component
                :is="nodeView(node.id)"
                :node="node"
                :is-dark="isDark"
                @slider="({ field, value }) => onSlider(node, field, value)"
                @choice="({ field, option, index }) => onChoice(node, field, option, index)"
                @toggle="({ field, on }) => onToggle(node, field, on)"
                @pick="markEdited(node)"
                @import-profile="(file) => importProfile(file)"
              />
            </div>
          </div>
        </div>
      </aside>
    </div>

    <div class="relative flex h-8 shrink-0 items-center gap-1 border-t px-1.5"
      :class="isDark ? 'border-white/10' : 'border-stone-900/10'">
      <button type="button"
        class="flex h-6 w-6 items-center justify-center rounded"
        :class="assessment
          ? 'bg-amber-200 text-amber-950'
          : (isDark ? 'text-stone-400 hover:bg-white/10 hover:text-stone-100' : 'text-stone-500 hover:bg-white hover:text-stone-800')"
        :title="t('grade.lightHint')"
        :aria-pressed="assessment"
        @click="assessment = !assessment">
        <IconBulb class="h-4 w-4" />
      </button>
      <div
        class="absolute left-1/2 top-1/2 flex h-5 w-12 -translate-x-1/2 -translate-y-1/2 cursor-row-resize touch-none items-center justify-center rounded-full"
        :class="isDark ? 'text-stone-500 hover:bg-white/10 hover:text-stone-200' : 'text-stone-400 hover:bg-stone-900/8 hover:text-stone-700'"
        role="separator"
        aria-orientation="horizontal"
        :aria-label="t('grade.stripResize')"
        :title="t('grade.stripResize')"
        :aria-valuenow="stripHeight"
        :aria-valuemin="STRIP_MIN"
        @pointerdown="onStripPointerDown"
        @pointermove="onStripPointerMove"
        @pointerup="endStripDrag"
        @pointercancel="endStripDrag">
        <svg viewBox="0 0 20 8" class="h-2 w-5" fill="currentColor" aria-hidden="true">
          <circle cx="4" cy="2" r="1.15" />
          <circle cx="10" cy="2" r="1.15" />
          <circle cx="16" cy="2" r="1.15" />
          <circle cx="4" cy="6" r="1.15" />
          <circle cx="10" cy="6" r="1.15" />
          <circle cx="16" cy="6" r="1.15" />
        </svg>
      </div>
    </div>

    <GradeFilmstrip
      ref="stripRef"
      :photos="photos"
      :current-path="photo?.path || ''"
      :thumb-for="thumbFor"
      :height="stripHeight"
      @select="pickPhoto"
      @load-more="emit('load-more')"
    />
  </div>
</template>
