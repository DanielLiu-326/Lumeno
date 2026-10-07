<script setup>
/**
 * VirtualGrid — 虚拟化网格。
 *
 * 缩放复刻苹果相册：以指针/双指中点所在行为锚，保持该行照片身份，
 * 其余行交叉淡变；用锚点块反推前方整行数与余数。贴头贴尾时回退为从头排列。
 */

import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick } from "vue";
import { VList } from "virtua/vue";
import {
  clamp,
  lerp,
  pinchZoomBrackets,
  snapLevelByCellWidth,
  buildGridRows,
  itemCell,
  cellIndex,
  planRowAnchor,
  destIndexAt,
  fromColToDestCol,
} from "../utils/latticeView.js";
import { isImageReady, prefetchMany, onImageReady, markImageReady, enqueue } from "../utils/imageThrottle.js";

const ZOOM_SMOOTH = 16;
const ZOOM_DONE_EPS = 0.003;
const ZOOM_SETTLE_MS = 40;
const OVERLAY_FADE_MS = 240;
const PHOTO_FADE_MS = 180;
const PHOTO_FADE_EASE = "cubic-bezier(0.37, 0, 0.63, 1)";
const TILE_SEAM = 1;
const LIST_BUFFER_PX = 1400;
const LIVE_TILE_MAX = 72;
const LIVE_SPAWN_PER_FRAME = 8;
const LIVE_RECT_MAX = 80;
const PINCH_WATCHDOG_MS = 6000;

const props = defineProps({
  items: { type: Array, required: true },
  itemKey: { type: [String, Function], required: true },
  gridClass: { type: String, required: true },
  zoomLevel: { type: Number, default: 0 },
  zoomClasses: { type: Array, default: () => [] },
  zoomGaps: { type: Array, default: () => [] },
  imageSrc: { type: Function, default: null },
  overscan: { type: Number, default: 2 },
  bottomThreshold: { type: Number, default: 500 },
  topInset: { type: Number, default: 0 },
  paused: { type: Boolean, default: false },
});

const emit = defineEmits(["scroll-near-bottom", "zooming"]);

const listRef = ref(null);
const hostRef = ref(null);
const flipLayerRef = ref(null);
const ghostHostRef = ref(null);
const overlayBoardRef = ref(null);
const edgeFillRef = ref(null);
const probeRef = ref(null);
const colCount = ref(1);
const headRemainder = ref(0);
const rowGapPx = ref(0);
const liveGridClass = ref(props.gridClass);
const probeWidthPx = ref(0);
const listItemSize = ref(80);
const listEpoch = ref(0);
const zooming = ref(false);
const livePinching = ref(false);
const interactionMode = ref("idle");
const scrollAnchorItemIndex = ref(0);
const listLayoutKey = computed(() =>
  `${colCount.value}|${headRemainder.value}|${liveGridClass.value}|${listEpoch.value}`,
);
const listBufferPx = LIST_BUFFER_PX;

let ro = null;
let resizeRaf = 0;
let resizeWhileZooming = false;
let nearBottomPending = false;
let fillCheckRaf = 0;
let fillRetryTimer = 0;
let relayoutAttempts = 0;
let scrollRestoreGen = 0;
let suppressScrollRestore = false;
let zoomGen = 0;
let zoomSession = null;
let settleTimer = 0;
let zoomRaf = 0;
let zoomLastTs = 0;
let zoomScale = 1;
let zoomScaleTo = 1;
let zoomT = 0;
let zoomFade = 0;
let pinchRaf = 0;
let pendingPinchFrame = null;
let pinchCommitGen = 0;
let pinchWatchdog = 0;
let zoomLayoutCache = null;
let zoomFocal = null;
const itemIndexMap = new Map();
let unsubImageReady = null;

function withVList(fn, fallback) {
  const vl = listRef.value;
  if (!vl) return fallback;
  try {
    return fn(vl);
  } catch {
    return fallback;
  }
}

function rowIndexOfItem(index) {
  const photoRow = itemCell(index, colCount.value, headRemainder.value).row;
  const extra = (Number(props.topInset) || 0) > 0 ? 1 : 0;
  if (extra && photoRow === 0) return 0;
  return photoRow + extra;
}

function updateScrollAnchor(offset) {
  withVList((vl) => {
    const y = Number.isFinite(offset) ? offset : (vl.scrollOffset ?? 0);
    if (typeof vl.findItemIndex !== "function") return;
    const rowIndex = vl.findItemIndex(y);
    if (rowIndex < 0) return;
    const row = rows.value[rowIndex];
    if (!row || row.kind === "inset") {
      scrollAnchorItemIndex.value = 0;
      return;
    }
    scrollAnchorItemIndex.value = row.startIndex;
  });
}

function restoreScrollAnchorSync() {
  if (props.items.length === 0) return;
  withVList((vl) => {
    if (typeof vl.scrollToIndex !== "function") return;
    const itemIdx = Math.min(
      scrollAnchorItemIndex.value,
      Math.max(0, props.items.length - 1),
    );
    vl.scrollToIndex(rowIndexOfItem(itemIdx), { align: "start" });
  });
}

function restoreScrollAnchor() {
  if (suppressScrollRestore || props.items.length === 0) return;
  const gen = ++scrollRestoreGen;
  nextTick(() => {
    requestAnimationFrame(() => {
      if (gen !== scrollRestoreGen) return;
      restoreScrollAnchorSync();
    });
  });
}

function countGridColumns(colsStr) {
  if (!colsStr || colsStr === "none") return 0;
  const repeat = colsStr.trim().match(/^repeat\(\s*(\d+)/i);
  if (repeat) return Number(repeat[1]) || 0;
  const tracks = [];
  let depth = 0;
  let current = "";
  for (const ch of colsStr) {
    if (ch === "(") depth += 1;
    else if (ch === ")") depth -= 1;
    if (/\s/.test(ch) && depth === 0) {
      if (current) tracks.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  if (current) tracks.push(current);
  return tracks.length;
}

function measureGrid(force = false) {
  if ((zooming.value || interactionMode.value !== "idle") && !force) {
    resizeWhileZooming = true;
    return;
  }
  if (!probeRef.value) return;
  syncProbeWidth();
  const style = window.getComputedStyle(probeRef.value);

  const colsStr = style.gridTemplateColumns;
  let nextColCount = colCount.value;
  const count = countGridColumns(colsStr);
  if (count > 0) nextColCount = count;

  const gap = parseFloat(style.rowGap);
  const nextRowGap = isNaN(gap) ? 0 : gap;

  const colsChanged = nextColCount !== colCount.value;
  const layoutChanged = colsChanged || nextRowGap !== rowGapPx.value;
  if (layoutChanged) updateScrollAnchor();
  if (colsChanged) headRemainder.value = 0;

  colCount.value = nextColCount;
  rowGapPx.value = nextRowGap;
  const rowW = measureRowWidth();
  if (rowW >= 8) {
    const nextSize = latticePitch(rowW, nextColCount, nextRowGap);
    if (nextSize >= 8) listItemSize.value = nextSize;
  }

  if (layoutChanged && !force) restoreScrollAnchor();
  if (layoutChanged) scheduleFillCheck();
}

function onHostResize() {
  if (props.paused) return;
  if (resizeRaf) return;
  resizeRaf = requestAnimationFrame(() => {
    resizeRaf = 0;
    if (props.paused) return;
    measureGrid();
  });
}

onMounted(() => {
  rebuildItemIndex();
  measureGrid();
  scheduleFillCheck();
  ro = new ResizeObserver(onHostResize);
  if (hostRef.value) ro.observe(hostRef.value);
  unsubImageReady = onImageReady(onThumbReady);
});

onBeforeUnmount(() => {
  scrollRestoreGen += 1;
  zoomGen += 1;
  suppressScrollRestore = true;
  unsubImageReady?.();
  unsubImageReady = null;
  window.clearTimeout(settleTimer);
  if (zoomRaf) cancelAnimationFrame(zoomRaf);
  zoomRaf = 0;
  if (resizeRaf) cancelAnimationFrame(resizeRaf);
  if (fillCheckRaf) cancelAnimationFrame(fillCheckRaf);
  fillCheckRaf = 0;
  window.clearTimeout(fillRetryTimer);
  if (pinchRaf) cancelAnimationFrame(pinchRaf);
  pinchRaf = 0;
  window.clearTimeout(pinchWatchdog);
  pinchWatchdog = 0;
  ro?.disconnect();
  ro = null;
  clearOverlay();
  clearEdgeTiles();
});

function peekProbeLayout() {
  if (!probeRef.value) return { cols: colCount.value, gap: rowGapPx.value };
  const style = getComputedStyle(probeRef.value);
  const cols = countGridColumns(style.gridTemplateColumns) || colCount.value;
  const gap = parseFloat(style.rowGap);
  return {
    cols: Math.max(1, cols),
    gap: Number.isFinite(gap) ? gap : 0,
  };
}

function applyProbeClass(className) {
  if (!probeRef.value || !className) return;
  probeRef.value.setAttribute(
    "class",
    `${className} pointer-events-none invisible absolute left-0 top-0`,
  );
}

function applyTargetLayout() {
  syncProbeWidth();
  applyProbeClass(props.gridClass);
  const { cols, gap } = peekProbeLayout();
  colCount.value = cols;
  rowGapPx.value = gap;
  liveGridClass.value = props.gridClass;
}

function peekLayoutForClass(className) {
  if (!probeRef.value || !className) {
    return { cols: colCount.value, gap: rowGapPx.value };
  }
  syncProbeWidth();
  const probe = probeRef.value;
  const originalClass = probe.getAttribute("class") || "";
  applyProbeClass(className);
  const layout = peekProbeLayout();
  probe.setAttribute("class", originalClass);
  return layout;
}

function applyOverlayTransform(scale) {
  const board = overlayBoardRef.value;
  if (!board || !zoomSession) return;
  board.style.transition = "none";
  board.style.transformOrigin = `${zoomSession.originX || 0}px ${zoomSession.originY || 0}px`;
  board.style.transform = `scale(${scale})`;
}

function resetBoardTransform() {
  const board = overlayBoardRef.value;
  if (!board) return;
  board.style.transition = "none";
  board.style.transform = "none";
  board.style.transformOrigin = "0 0";
}

function writeGhostRect(el, x, y, w, h) {
  el.dataset.x = String(x);
  el.dataset.y = String(y);
  el.dataset.w = String(w);
  el.dataset.h = String(h);
  el.style.width = `${w}px`;
  el.style.height = `${h}px`;
  el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
}

function readGhostRect(el) {
  return {
    x: Number(el.dataset.x),
    y: Number(el.dataset.y),
    w: Number(el.dataset.w),
    h: Number(el.dataset.h),
  };
}

function relockSessionFromOverlay() {
  const session = zoomSession;
  if (!session) return;
  bakeOverlayFrom();
  if (!session.targetCols || !session.plan) return;
  const keepIndex = destKeepIndex(session.plan, session.anchorIndex);
  session.frozenCols = session.targetCols;
  session.frozenHead = session.plan.headRemainder;
  session.frozenGap = session.targetGap;
  session.frozenPitch = session.targetPitch || session.frozenPitch;
  session.frozenCell = cellSize(
    session.rowW || measureRowWidth(),
    session.frozenCols,
    session.frozenGap,
  );
  session.fromAnchorRow = itemCell(
    session.anchorIndex,
    session.frozenCols,
    session.frozenHead,
  ).row;
  if (session.fromAnchor) {
    session.anchorRowY = session.fromAnchor.y;
    if (keepIndex >= 0) {
      const destCol = (session.plan.keepStartCol || 0) + keepIndex;
      session.gridOriginX = session.fromAnchor.x - destCol * Math.max(1, session.frozenPitch);
    }
  }
}

function bakeOverlayFrom() {
  const board = overlayBoardRef.value;
  const session = zoomSession;
  if (!board || !session) return;
  const focalId = itemIdAt(session.anchorIndex);
  let focalRect = null;
  for (const el of board.children) {
    const rect = readGhostRect(el);
    if (!Number.isFinite(rect.x) || !Number.isFinite(rect.y)) continue;
    writeGhostRect(el, rect.x, rect.y, rect.w, rect.h);
    el.dataset.fromX = String(rect.x);
    el.dataset.fromY = String(rect.y);
    el.dataset.fromW = String(rect.w);
    el.dataset.fromH = String(rect.h);
    delete el.dataset.toX;
    delete el.dataset.toY;
    delete el.dataset.toW;
    delete el.dataset.toH;
    if (el.dataset.zoomId === focalId) focalRect = rect;
  }
  if (focalRect) {
    session.fromAnchor = {
      id: focalId,
      x: focalRect.x,
      y: focalRect.y,
      w: focalRect.w,
      h: focalRect.h,
    };
    session.anchorRowY = focalRect.y;
  }
  zoomT = 0;
  resetBoardTransform();
}

function destKeepIndex(plan, itemIndex) {
  const keep = plan?.keepItems || [];
  return keep.indexOf(itemIndex);
}

function theoreticalKeepRect(session, keepIndex, destOriginX) {
  const plan = session.plan;
  const pitch = Math.max(1, session.targetPitch || session.frozenPitch);
  const cell = Math.max(1, cellSize(
    session.rowW || measureRowWidth(),
    session.targetCols,
    session.targetGap,
  ));
  const destCol = (plan.keepStartCol || 0) + keepIndex;
  return {
    x: destOriginX + destCol * pitch,
    y: session.anchorRowY,
    w: cell,
    h: cell,
  };
}

function assignTileDests(session) {
  const board = overlayBoardRef.value;
  const plan = session?.plan;
  if (!board || !plan) return false;
  const destCards = captureVisibleCards(flipLayerRef.value);
  const destById = new Map(destCards.map((card) => [card.id, card]));
  const keepIds = new Set((plan.keepItems || []).map((index) => itemIdAt(index)));
  let destOriginX = session.gridOriginX || 0;
  for (const itemIndex of plan.keepItems || []) {
    const card = destById.get(itemIdAt(itemIndex));
    if (!card) continue;
    const keepIndex = destKeepIndex(plan, itemIndex);
    if (keepIndex < 0) continue;
    destOriginX = card.x - ((plan.keepStartCol || 0) + keepIndex) * Math.max(1, session.targetPitch);
    break;
  }
  let moving = false;
  for (const el of board.children) {
    const id = el.dataset.zoomId || "";
    const itemIndex = itemIndexByKey(id);
    const keepIndex = destKeepIndex(plan, itemIndex);
    const keep = keepIds.has(id) && keepIndex >= 0;
    el.dataset.keep = keep ? "1" : "0";
    const current = readGhostRect(el);
    el.dataset.fromX = String(current.x);
    el.dataset.fromY = String(current.y);
    el.dataset.fromW = String(current.w);
    el.dataset.fromH = String(current.h);
    if (!keep) {
      delete el.dataset.toX;
      el.style.transition = `opacity ${OVERLAY_FADE_MS}ms ${PHOTO_FADE_EASE}`;
      el.style.opacity = "0";
      continue;
    }
    const measured = destById.get(id);
    const dest = measured && measured.w >= 2
      ? measured
      : theoreticalKeepRect(session, keepIndex, destOriginX);
    el.dataset.toX = String(dest.x);
    el.dataset.toY = String(dest.y);
    el.dataset.toW = String(dest.w + TILE_SEAM);
    el.dataset.toH = String(dest.h + TILE_SEAM);
    el.style.transition = "none";
    el.style.opacity = "1";
    if (Math.abs(dest.x - current.x) > 1
      || Math.abs(dest.y - current.y) > 1
      || Math.abs(dest.w - current.w) > 1
      || Math.abs(dest.h - current.h) > 1) {
      moving = true;
    }
  }
  setDestKeepHidden(true);
  return moving;
}

function setDestKeepHidden(hidden) {
  const layer = flipLayerRef.value;
  const plan = zoomSession?.plan;
  if (!layer || !plan) return;
  const keep = new Set((plan.keepItems || []).map((index) => itemIdAt(index)));
  for (const el of layer.querySelectorAll("[data-flip-id]")) {
    const id = el.getAttribute("data-flip-id") || "";
    if (!keep.has(id)) continue;
    el.style.transition = "none";
    el.style.opacity = hidden ? "0" : "";
  }
}

function paintFlipTiles(t) {
  const board = overlayBoardRef.value;
  if (!board) return;
  const amount = Math.max(0, Math.min(1, t));
  for (const el of board.children) {
    const fx = Number(el.dataset.fromX);
    const fy = Number(el.dataset.fromY);
    const fw = Number(el.dataset.fromW);
    const fh = Number(el.dataset.fromH);
    if (!Number.isFinite(fx) || !Number.isFinite(fy)) continue;
    if (el.dataset.keep === "1" && el.dataset.toX != null) {
      const dx = Number(el.dataset.toX);
      const dy = Number(el.dataset.toY);
      const dw = Number(el.dataset.toW);
      const dh = Number(el.dataset.toH);
      writeGhostRect(
        el,
        fx + (dx - fx) * amount,
        fy + (dy - fy) * amount,
        fw + (dw - fw) * amount,
        fh + (dh - fh) * amount,
      );
    } else {
      writeGhostRect(el, fx, fy, fw, fh);
    }
  }
}

function paintBoardFrame() {
  if (interactionMode.value === "pinching") {
    applyOverlayTransform(zoomScale);
    return;
  }
  paintFlipTiles(zoomT);
}

function zoomFadeT() {
  return Math.max(0, Math.min(1, zoomFade));
}

function ensureDestOverlay(box, destSrc) {
  if (!box || !destSrc) return null;
  let overlay = box.querySelector("[data-zoom-dest]");
  if (overlay) {
    const shown = overlay.currentSrc || overlay.src || "";
    if (!urlsMatch(shown, destSrc)) {
      overlay.src = destSrc;
      overlay.style.opacity = "0";
    }
    return overlay;
  }
  overlay = bindCoverImage(destSrc, { fade: false, role: "new" });
  if (!overlay) return null;
  overlay.dataset.zoomDest = "";
  overlay.style.zIndex = "2";
  overlay.style.opacity = "0";
  overlay.style.transition = "none";
  overlay.style.pointerEvents = "none";
  box.appendChild(overlay);
  beginDestLoad(destSrc);
  return overlay;
}

function attachCellCrossfades(session) {
  const layer = flipLayerRef.value;
  const plan = session?.plan;
  if (!layer || !plan) return;
  const nFrom = Math.max(1, session.frozenCols);
  const urls = [];
  for (const el of layer.querySelectorAll("[data-flip-id]")) {
    const id = el.getAttribute("data-flip-id") || "";
    const fromIndex = itemIndexByKey(id);
    if (fromIndex < 0) continue;
    const from = itemCell(fromIndex, nFrom, session.frozenHead);
    const destCol = fromColToDestCol(plan, from.col);
    const box = el.querySelector("[data-photo-image]") || el;
    const destIndex = destCol < 0
      ? -1
      : destIndexAt(
        plan,
        plan.destAnchorRow + (from.row - session.fromAnchorRow),
        destCol,
        props.items.length,
      );
    if (destCol < 0 || destIndex < 0) {
      el.dataset.zoomFade = "leave";
      box.querySelector("[data-zoom-dest]")?.remove();
      continue;
    }
    const destId = itemIdAt(destIndex);
    const destSrc = destId ? srcForId(destId) : "";
    if (destId === id) {
      el.dataset.zoomFade = "keep";
      box.querySelector("[data-zoom-dest]")?.remove();
      continue;
    }
    el.dataset.zoomFade = "swap";
    urls.push(destSrc);
    ensureDestOverlay(box, destSrc);
  }
  warmDestUrls(urls);
}

function applyZoomFades(t) {
  const amount = Math.max(0, Math.min(1, t));
  const layer = flipLayerRef.value;
  if (layer) {
    for (const el of layer.querySelectorAll("[data-zoom-fade=leave]")) {
      el.style.opacity = String(1 - amount);
    }
    for (const el of layer.querySelectorAll("[data-zoom-fade=keep]")) {
      el.style.opacity = "1";
    }
    for (const el of layer.querySelectorAll("[data-zoom-fade=swap]")) {
      el.style.opacity = "1";
      const box = el.querySelector("[data-photo-image]") || el;
      for (const img of box.querySelectorAll("img")) {
        img.style.transition = "none";
        img.style.opacity = img.hasAttribute("data-zoom-dest")
          ? String(amount)
          : String(1 - amount);
      }
    }
  }
  const edges = edgeFillRef.value;
  if (edges) {
    for (const el of edges.children) {
      el.style.transition = "none";
      el.style.opacity = String(amount);
    }
  }
}

function clearZoomFades() {
  const layer = flipLayerRef.value;
  if (!layer) return;
  for (const el of layer.querySelectorAll("[data-flip-id]")) {
    if (el.style.opacity === "0") el.style.opacity = "";
  }
  for (const el of layer.querySelectorAll("[data-zoom-fade]")) {
    const box = el.querySelector("[data-photo-image]") || el;
    for (const img of box.querySelectorAll("img")) {
      if (!img.hasAttribute("data-zoom-dest")) img.style.opacity = "";
    }
    delete el.dataset.zoomFade;
    el.style.opacity = "";
  }
  for (const img of layer.querySelectorAll("[data-zoom-dest]")) img.remove();
}

function stopBoardZoomLoop() {
  if (zoomRaf) cancelAnimationFrame(zoomRaf);
  zoomRaf = 0;
  zoomLastTs = 0;
}

function tickBoardZoom(ts) {
  zoomRaf = 0;
  if (!zoomSession || !zooming.value) return;
  if (!zoomLastTs) zoomLastTs = ts;
  const dt = Math.min(0.048, (ts - zoomLastTs) / 1000);
  zoomLastTs = ts;
  const k = 1 - Math.exp(-ZOOM_SMOOTH * dt);
  zoomT += (1 - zoomT) * k;
  paintFlipTiles(zoomT);
  if (1 - zoomT > ZOOM_DONE_EPS) {
    zoomRaf = requestAnimationFrame(tickBoardZoom);
    return;
  }
  zoomT = 1;
  paintFlipTiles(1);
  scheduleZoomCommit();
}

function ensureBoardZoomLoop() {
  if (!zoomRaf) {
    zoomLastTs = 0;
    zoomRaf = requestAnimationFrame(tickBoardZoom);
  }
}

function scheduleZoomCommit() {
  window.clearTimeout(settleTimer);
  settleTimer = window.setTimeout(() => {
    commitGridZoom();
  }, ZOOM_SETTLE_MS);
}

function resolveAnchorRowY(anchorIndex, cards) {
  const id = itemIdAt(anchorIndex);
  const direct = cards.find((card) => card.id === id);
  if (direct) return direct.y;
  const { focal } = readZoomFocal(cards);
  let best = null;
  for (const card of cards) {
    const cy = card.y + card.h * 0.5;
    const dist = Math.abs(cy - focal.y);
    if (!best || dist < best.dist) best = { y: card.y, dist };
  }
  if (best) return best.y;
  return focal.y;
}

function measureContentWidth() {
  const scroller = flipLayerRef.value?.firstElementChild;
  if (scroller?.clientWidth > 0) return scroller.clientWidth;
  const row = flipLayerRef.value?.querySelector(".grid");
  if (row?.clientWidth > 0) return row.clientWidth;
  return hostRef.value?.clientWidth || 0;
}

function measureRowWidth() {
  const row = flipLayerRef.value?.querySelector(".grid");
  if (row?.clientWidth > 0) return row.clientWidth;
  return measureContentWidth();
}

function syncProbeWidth() {
  const width = measureContentWidth();
  if (width <= 0) return 0;
  probeWidthPx.value = width;
  if (probeRef.value) probeRef.value.style.width = `${width}px`;
  return width;
}

function rebuildItemIndex() {
  itemIndexMap.clear();
  const items = props.items;
  for (let i = 0; i < items.length; i += 1) {
    itemIndexMap.set(String(getKey(items[i])), i);
  }
}

function itemIndexByKey(id) {
  if (!id) return -1;
  const hit = itemIndexMap.get(String(id));
  return hit == null ? -1 : hit;
}

function srcForId(id) {
  const index = itemIndexByKey(id);
  if (index < 0) return "";
  return props.imageSrc?.(props.items[index]) || "";
}

function thumbSrcFromImg(img, fallback = "") {
  if (!img) return fallback;
  const src = img.currentSrc || img.src || "";
  return src || fallback;
}

function warmDestUrls(urls) {
  prefetchMany([...new Set((urls || []).filter(Boolean))], "high");
}

function collectPlanViewportUrls(session, extraRows = 10) {
  const plan = session?.plan;
  if (!plan) return [];
  const pitch = Math.max(1, session.targetPitch || session.frozenPitch);
  const hostH = hostRef.value?.clientHeight || 800;
  const minDr = Math.floor(-Math.max(0, session.anchorRowY) / pitch) - 2;
  const maxDr = Math.ceil((hostH - Math.max(0, session.anchorRowY)) / pitch) + extraRows;
  const urls = [];
  for (let dr = minDr; dr <= maxDr; dr += 1) {
    for (let c = 0; c < plan.destCols; c += 1) {
      const index = destIndexAt(plan, plan.destAnchorRow + dr, c, props.items.length);
      if (index < 0) continue;
      const src = props.imageSrc?.(props.items[index]);
      if (src) urls.push(src);
    }
  }
  return urls;
}

function prefetchTargetViewport(session, extraRows = 12) {
  warmDestUrls(collectPlanViewportUrls(session, extraRows));
}

function clearEdgeTiles() {
  edgeFillRef.value?.replaceChildren();
}

function paintZoomEdgeTiles(session, scale) {
  const host = edgeFillRef.value;
  const plan = session?.plan;
  if (!host || !plan) return;
  const pitch = Math.max(1, session.frozenPitch);
  const cell = Math.max(1, session.frozenCell || pitch);
  const s = Math.max(0.2, Math.min(Number(scale) || 1, 1));
  const originX = Number.isFinite(session.gridOriginX) ? session.gridOriginX : 0;
  const ox = Number.isFinite(session.originX) ? session.originX : 0;
  const oy = Number.isFinite(session.originY) ? session.originY : 0;
  const hostW = hostRef.value?.clientWidth || 0;
  const hostH = hostRef.value?.clientHeight || 0;
  const localLeft = ox + (0 - ox) / s;
  const localRight = ox + (hostW - ox) / s;
  const localTop = oy + (0 - oy) / s;
  const localBottom = oy + (hostH - oy) / s;
  const extra = 2;
  const minDr = Math.floor((localTop - session.anchorRowY) / pitch) - extra;
  const maxDr = Math.ceil((localBottom - session.anchorRowY) / pitch) + extra;
  const fromIds = new Set();
  for (const el of flipLayerRef.value?.querySelectorAll("[data-flip-id]") || []) {
    fromIds.add(el.getAttribute("data-flip-id") || "");
  }
  const existing = new Map();
  for (const el of [...host.children]) {
    existing.set(`${el.dataset.zoomCol}:${el.dataset.zoomDr}`, el);
  }
  const urls = [];
  const keep = new Set();
  const nFrom = Math.max(1, session.frozenCols);

  for (let dr = minDr; dr <= maxDr; dr += 1) {
    for (let dc = 0; dc < plan.destCols; dc += 1) {
      const destIndex = destIndexAt(
        plan,
        plan.destAnchorRow + dr,
        dc,
        props.items.length,
      );
      if (destIndex < 0) continue;
      const virtualFromCol = dc - plan.keepStartCol + (plan.keepFromCol0 || 0);
      if (virtualFromCol >= 0 && virtualFromCol < nFrom) {
        const fromIndex = cellIndex(
          session.fromAnchorRow + dr,
          virtualFromCol,
          nFrom,
          session.frozenHead,
          props.items.length,
        );
        if (fromIndex >= 0 && fromIds.has(itemIdAt(fromIndex))) continue;
      }
      const x = originX + virtualFromCol * pitch;
      const y = session.anchorRowY + dr * pitch;
      if (x > localRight + cell || x + cell < localLeft - cell) continue;
      if (y > localBottom + cell || y + cell < localTop - cell) continue;
      const id = itemIdAt(destIndex);
      const src = srcForId(id);
      if (src) urls.push(src);
      const key = `${dc}:${dr}`;
      keep.add(key);
      let el = existing.get(key);
      if (el) {
        el.style.transition = "none";
        el.style.width = `${cell + TILE_SEAM}px`;
        el.style.height = `${cell + TILE_SEAM}px`;
        el.style.transform = `translate3d(${x}px, ${y}px, 0)`;
        el.dataset.zoomId = id;
        if (src && el.dataset.wantSrc !== src) {
          el.replaceChildren();
          el.dataset.wantSrc = src;
          attachGhostPhoto(el, src);
        }
        existing.delete(key);
        continue;
      }
      el = makeGhostBox({ x, y, w: cell + TILE_SEAM, h: cell + TILE_SEAM, id });
      el.dataset.zoomCol = String(dc);
      el.dataset.zoomDr = String(dr);
      el.style.opacity = "0";
      if (src) {
        el.dataset.wantSrc = src;
        beginDestLoad(src);
        attachGhostPhoto(el, src);
      }
      host.appendChild(el);
    }
  }
  for (const [key, el] of existing) {
    if (!keep.has(key)) el.remove();
  }
  warmDestUrls(urls);
}

function urlsMatch(a, b) {
  if (!a || !b) return false;
  if (a === b) return true;
  try {
    return new URL(a, window.location.href).href === new URL(b, window.location.href).href;
  } catch {
    return false;
  }
}

function fillGhostImage(el, url) {
  if (!el || !url) return;
  const images = el.querySelectorAll("img");
  for (const existing of images) {
    const shown = existing.currentSrc || existing.src || "";
    if (urlsMatch(shown, url)) {
      if ((existing.complete && existing.naturalWidth > 1) || isImageReady(url)) {
        existing.style.opacity = "1";
      }
      return;
    }
  }
  const img = bindCoverImage(url, { fade: true, role: "new" });
  if (!img) return;
  el.appendChild(img);
  revealDestImage(img);
}

function onThumbReady(url) {
  if (!url) return;
  if (!zooming.value && interactionMode.value === "idle") return;
  const edges = edgeFillRef.value;
  if (edges) {
    for (const el of edges.children) {
      if (el.dataset.wantSrc === url) fillGhostImage(el, url);
    }
  }
  const layer = flipLayerRef.value;
  if (layer) {
    for (const img of layer.querySelectorAll("[data-zoom-dest]")) {
      if (urlsMatch(img.currentSrc || img.src, url) || urlsMatch(img.src, url)) {
        img.style.opacity = String(zoomFadeT());
      }
    }
  }
  applyZoomFades(zoomFadeT());
}

function getHostRect() {
  return hostRef.value?.getBoundingClientRect() ?? null;
}

function isZoomBusy() {
  return zooming.value || interactionMode.value !== "idle";
}

function clearLayerZoom() {
  const layer = flipLayerRef.value;
  if (!layer) return;
  layer.style.transition = "none";
  layer.style.transform = "none";
  layer.style.transformOrigin = "";
  layer.style.willChange = "";
  layer.style.visibility = "";
  layer.style.opacity = "1";
}

function waitFrames(count = 2) {
  return new Promise((resolve) => {
    const step = (left) => {
      if (left <= 0) resolve();
      else requestAnimationFrame(() => step(left - 1));
    };
    step(count);
  });
}

function captureVisibleCards(root) {
  const host = hostRef.value;
  if (!host || !root) return [];
  const hostRect = host.getBoundingClientRect();
  if (hostRect.width < 2 || hostRect.height < 2) return [];
  const cards = root.querySelectorAll("[data-flip-id]");
  const out = [];
  for (const el of cards) {
    const box = el.querySelector("[data-photo-image]") || el;
    const rect = box.getBoundingClientRect();
    if (rect.width < 2 || rect.height < 2) continue;
    if (rect.bottom < hostRect.top - 800 || rect.top > hostRect.bottom + 800) continue;
    const img = el.querySelector("img");
    out.push({
      id: el.getAttribute("data-flip-id") || "",
      x: rect.left - hostRect.left,
      y: rect.top - hostRect.top,
      w: Math.max(rect.width, 1),
      h: Math.max(rect.height, 1),
      src: thumbSrcFromImg(img),
    });
  }
  return out;
}

function latticePitch(rowW, cols, gap = 0) {
  const n = Math.max(1, cols);
  const g = Math.max(0, Number(gap) || 0);
  return Math.max(1, ((Number(rowW) || 0) + g) / n);
}

function cellSize(rowW, cols, gap) {
  const n = Math.max(1, cols);
  return (rowW - Math.max(0, gap) * (n - 1)) / n;
}

function median(values) {
  const list = values.filter((value) => Number.isFinite(value) && value > 0).sort((a, b) => a - b);
  if (!list.length) return 0;
  return list[Math.floor(list.length / 2)];
}

function measureFrozenLattice(cards, cols) {
  const n = Math.max(1, cols);
  const rowW = measureRowWidth();
  const fallbackPitch = latticePitch(rowW, n, rowGapPx.value);
  const fallbackCell = cellSize(rowW, n, rowGapPx.value);
  if (!cards.length) return { originX: 0, pitch: fallbackPitch, cell: fallbackCell };
  const rows = [];
  const sorted = [...cards].sort((a, b) => a.y - b.y || a.x - b.x);
  for (const card of sorted) {
    const row = rows.find((item) => Math.abs(item.y - card.y) < Math.max(8, card.h * 0.25));
    if (row) row.cards.push(card);
    else rows.push({ y: card.y, cards: [card] });
  }
  const pitches = [];
  for (const row of rows) {
    const xs = row.cards.sort((a, b) => a.x - b.x);
    for (let i = 1; i < xs.length; i += 1) {
      const delta = xs[i].x - xs[i - 1].x;
      if (delta > 8) pitches.push(delta);
    }
  }
  const pitch = median(pitches) || fallbackPitch;
  const origins = [];
  for (const card of cards) {
    const idx = itemIndexByKey(card.id);
    if (idx < 0) continue;
    const col = itemCell(idx, n, headRemainder.value).col;
    origins.push(card.x - col * pitch);
  }
  origins.sort((a, b) => a - b);
  const originX = origins.length ? origins[Math.floor(origins.length / 2)] : 0;
  const cell = median(cards.map((card) => card.w)) || fallbackCell;
  return { originX, pitch, cell };
}

function makeGhostBox(rect) {
  const el = document.createElement("div");
  el.dataset.zoomGhost = "";
  if (rect.id) el.dataset.zoomId = rect.id;
  el.style.cssText = [
    "position:absolute",
    "left:0",
    "top:0",
    "overflow:hidden",
    "will-change:transform,opacity",
    "backface-visibility:hidden",
    "background:transparent",
  ].join(";");
  writeGhostRect(el, rect.x, rect.y, rect.w, rect.h);
  return el;
}

function bindCoverImage(src, { fade = false, role = "keep" } = {}) {
  const url = typeof src === "string" ? src : "";
  if (!url) return null;
  const img = document.createElement("img");
  img.alt = "";
  img.draggable = false;
  img.fetchPriority = "high";
  img.decoding = "async";
  img.dataset.zoomImg = role;
  img.style.cssText = [
    "position:absolute",
    "inset:0",
    "width:100%",
    "height:100%",
    "object-fit:cover",
    "display:block",
    fade ? "opacity:0" : "opacity:1",
  ].join(";");
  const onReady = () => {
    markImageReady(url);
    if (fade) revealDestImage(img);
  };
  img.addEventListener("load", onReady, { once: true });
  img.src = url;
  if (img.complete && img.naturalWidth > 1) {
    markImageReady(url);
    if (fade) revealDestImage(img);
  }
  return img;
}

function revealDestImage(img) {
  if (!img) return;
  if (!(img.complete && img.naturalWidth > 1) && !isImageReady(img.currentSrc || img.src)) return;
  img.style.transition = `opacity ${PHOTO_FADE_MS}ms ${PHOTO_FADE_EASE}`;
  requestAnimationFrame(() => { img.style.opacity = "1"; });
}

function beginDestLoad(url) {
  if (!url || isImageReady(url)) return;
  enqueue(url, undefined, { priority: "high" }).catch(() => {});
}

function attachGhostPhoto(el, src) {
  if (!src) return null;
  const img = bindCoverImage(src, { role: "keep" });
  if (!img) return null;
  el.appendChild(img);
  return img;
}

function clearOverlay() {
  const board = overlayBoardRef.value;
  if (board) {
    board.replaceChildren();
    board.style.transform = "none";
    board.style.transformOrigin = "";
    board.style.willChange = "";
  }
}

function setListHidden(hidden) {
  const layer = flipLayerRef.value;
  if (!layer) return;
  layer.style.visibility = hidden ? "hidden" : "";
}

function mountFromOverlay(cards) {
  const board = overlayBoardRef.value;
  const layer = flipLayerRef.value;
  if (!board) return;
  board.replaceChildren();
  board.style.willChange = "transform";
  const liveImgs = new Map();
  if (layer) {
    for (const el of layer.querySelectorAll("[data-flip-id]")) {
      const id = el.getAttribute("data-flip-id");
      const img = el.querySelector("img");
      if (id && img) liveImgs.set(id, img);
    }
  }
  for (const card of cards) {
    const el = makeGhostBox({
      ...card,
      w: card.w + TILE_SEAM,
      h: card.h + TILE_SEAM,
    });
    const live = liveImgs.get(card.id);
    if (live && live.complete && live.naturalWidth > 1) {
      const img = live.cloneNode(false);
      img.alt = "";
      img.draggable = false;
      img.style.cssText = [
        "position:absolute",
        "inset:0",
        "width:100%",
        "height:100%",
        "object-fit:cover",
        "display:block",
        "opacity:1",
      ].join(";");
      el.appendChild(img);
    } else if (card.src) {
      attachGhostPhoto(el, card.src);
    }
    board.appendChild(el);
    el.dataset.fromX = String(card.x);
    el.dataset.fromY = String(card.y);
    el.dataset.fromW = String(card.w + TILE_SEAM);
    el.dataset.fromH = String(card.h + TILE_SEAM);
  }
}

function markOverlayRoles(session) {
  assignTileDests(session);
}

async function revealDestUnderneath(session, gen, { showList = true } = {}) {
  const pitch = Math.max(
    8,
    session.targetPitch || latticePitch(measureRowWidth(), session.targetCols, session.targetGap),
  );
  listItemSize.value = pitch;
  liveGridClass.value = session.targetGridClass;
  colCount.value = session.targetCols;
  rowGapPx.value = session.targetGap;
  headRemainder.value = session.plan.headRemainder;
  scrollAnchorItemIndex.value = Math.max(0, session.anchorIndex);
  setListHidden(!showList);
  scheduleFillCheck();
  await nextTick();
  if (gen != null && gen !== zoomGen) return;
  await waitFrames(2);
  if (gen != null && gen !== zoomGen) return;
  const holdY = session.fromAnchor?.y ?? session.anchorRowY;
  await scrollItemToY(session.anchorIndex, holdY);
  if (gen != null && gen !== zoomGen) return;
  await waitFrames(1);
  if (gen != null && gen !== zoomGen) return;
  scheduleFillCheck();
}

function findDestCard(session) {
  const from = session?.fromAnchor;
  const destCards = captureVisibleCards(flipLayerRef.value);
  if (from?.id) {
    const hit = destCards.find((card) => card.id === from.id);
    if (hit) return hit;
  }
  return destCards.find((card) => card.id === itemIdAt(session.anchorIndex)) || null;
}

async function waitDestCoverage(gen) {
  const hostH = hostRef.value?.clientHeight || 0;
  if (hostH < 8) return;
  for (let i = 0; i < 3; i += 1) {
    if (gen != null && gen !== zoomGen) return;
    scheduleFillCheck();
    const dest = findDestCard(zoomSession);
    const bottom = contentBottomPx();
    if (dest && bottom >= Math.min(hostH * 0.72, hostH - 48)) return;
    await waitFrames(1);
  }
}

async function finalizeDestToOverlay(session) {
  const dest = findDestCard(session);
  const board = overlayBoardRef.value;
  if (!dest || !board) return;
  const ghost = [...board.children].find((el) => el.dataset.zoomId === dest.id);
  const endY = ghost ? Number(ghost.dataset.y) : dest.y;
  if (!Number.isFinite(endY)) return;
  if (Math.abs(dest.y - endY) > 0.5) {
    const moved = withVList((vl) => {
      if (typeof vl.scrollBy !== "function") return false;
      vl.scrollBy(dest.y - endY);
      return true;
    });
    if (moved) await waitFrames(1);
  }
}

function nearestCardAt(cards, x, y) {
  const direct = cards.find((card) => (
    x >= card.x && x <= card.x + card.w && y >= card.y && y <= card.y + card.h
  ));
  if (direct) return direct;
  return cards.reduce((nearest, card) => {
    const dx = x - (card.x + card.w / 2);
    const dy = y - (card.y + card.h / 2);
    const distance = dx * dx + dy * dy;
    return !nearest || distance < nearest.distance ? { card, distance } : nearest;
  }, null)?.card || null;
}

function readZoomLayouts() {
  const width = measureRowWidth();
  const cacheKey = `${width}|${props.zoomClasses.join("|")}|${props.zoomGaps.join(",")}`;
  if (zoomLayoutCache?.key === cacheKey) return zoomLayoutCache.layouts;
  const probe = probeRef.value;
  if (!probe || !props.zoomClasses.length) {
    const layouts = [{ cols: colCount.value, gap: rowGapPx.value }];
    zoomLayoutCache = { key: cacheKey, layouts };
    return layouts;
  }
  const originalClass = probe.getAttribute("class") || "";
  const originalWidth = probe.style.width;
  const layouts = props.zoomClasses.map((className, index) => {
    probe.setAttribute(
      "class",
      `${className} pointer-events-none invisible absolute left-0 top-0`,
    );
    probe.style.width = `${width}px`;
    const style = getComputedStyle(probe);
    return {
      cols: Math.max(1, countGridColumns(style.gridTemplateColumns) || colCount.value),
      gap: Math.max(0, Number(props.zoomGaps[index]) || 0),
    };
  });
  probe.setAttribute("class", originalClass);
  probe.style.width = originalWidth;
  zoomLayoutCache = { key: cacheKey, layouts };
  return layouts;
}

async function scrollItemToY(itemIndex, desiredY) {
  const item = props.items[itemIndex];
  if (!item) return;
  const id = String(getKey(item));
  for (let attempt = 0; attempt < 4; attempt += 1) {
    const scrolled = withVList((vl) => {
      if (typeof vl.scrollToIndex !== "function") return false;
      vl.scrollToIndex(rowIndexOfItem(Math.max(0, itemIndex)), { align: "start" });
      return true;
    });
    if (!scrolled) return;
    await waitFrames(1);
    const cards = captureVisibleCards(flipLayerRef.value);
    const card = cards.find((item) => item.id === id);
    if (!card) continue;
    withVList((vl) => {
      if (typeof vl.scrollBy !== "function") return;
      vl.scrollBy(card.y - desiredY);
    });
    await waitFrames(1);
    return;
  }
}

function setZoomFocal(x, y) {
  if (!Number.isFinite(x) || !Number.isFinite(y)) return;
  zoomFocal = { x, y };
}

function readZoomFocal(cards = []) {
  const host = hostRef.value;
  const fallback = {
    x: (host?.clientWidth || 0) * 0.5,
    y: (host?.clientHeight || 0) * 0.5,
  };
  const focal = zoomFocal || fallback;
  const anchor = nearestCardAt(cards, focal.x, focal.y) || cards[0] || null;
  return { focal, anchor };
}

function itemIdAt(index) {
  if (index < 0 || index >= props.items.length) return "";
  return String(getKey(props.items[index]));
}

function captureZoomFrom(focalOverride = null) {
  const layer = flipLayerRef.value;
  const host = hostRef.value;
  if (!layer || !host) return null;
  updateScrollAnchor();
  const cards = captureVisibleCards(layer);
  if (!cards.length) return null;
  const { focal, anchor } = focalOverride
    ? {
      focal: focalOverride,
      anchor: nearestCardAt(cards, focalOverride.x, focalOverride.y) || cards[0] || null,
    }
    : readZoomFocal(cards);
  const anchorIndex = anchor
    ? itemIndexByKey(anchor.id)
    : scrollAnchorItemIndex.value;
  if (anchorIndex < 0) return null;
  const lattice = measureFrozenLattice(cards, colCount.value);
  const fromCell = itemCell(anchorIndex, colCount.value, headRemainder.value);
  return {
    frozenGridClass: liveGridClass.value,
    frozenCols: colCount.value,
    frozenHead: headRemainder.value,
    frozenGap: rowGapPx.value,
    frozenPitch: lattice.pitch,
    frozenCell: lattice.cell,
    gridOriginX: lattice.originX,
    fromAnchorRow: fromCell.row,
    fromAnchor: anchor
      ? { id: anchor.id, x: anchor.x, y: anchor.y, w: anchor.w, h: anchor.h }
      : null,
    anchorIndex: Math.max(0, anchorIndex),
    anchorRowY: resolveAnchorRowY(Math.max(0, anchorIndex), cards),
    focalX: focal.x,
    focalY: focal.y,
    startLevel: props.zoomLevel,
    rowW: measureRowWidth(),
  };
}

function applyDestPlan(session, destLayout) {
  const destPitch = latticePitch(session.rowW || measureRowWidth(), destLayout.cols, destLayout.gap);
  const plan = planRowAnchor({
    total: props.items.length,
    fromCols: session.frozenCols,
    fromHeadRemainder: session.frozenHead,
    destCols: destLayout.cols,
    focalIndex: session.anchorIndex,
    viewportH: hostRef.value?.clientHeight || 0,
    destPitch,
    destCellH: destPitch,
    anchorRowY: session.anchorRowY,
  });
  session.plan = plan;
  session.targetGridClass = props.gridClass;
  session.targetCols = destLayout.cols;
  session.targetGap = destLayout.gap;
  session.targetPitch = destPitch;
  session.handoffY = session.fromAnchor?.y ?? session.anchorRowY;
}

function retargetZoomFades(session) {
  prefetchTargetViewport(session, 24);
  markOverlayRoles(session);
}

async function startGridZoom() {
  const gen = ++zoomGen;
  window.clearTimeout(settleTimer);
  scrollRestoreGen += 1;
  suppressScrollRestore = true;
  let handedOff = false;

  try {
    if (!zoomSession) {
      const captured = captureZoomFrom();
      if (!captured) {
        liveGridClass.value = props.gridClass;
        return;
      }
      const cards = captureVisibleCards(flipLayerRef.value);
      zoomSession = captured;
      zoomScale = 1;
      zoomT = 0;
      zoomFade = 0;
      mountFromOverlay(cards);
      resetBoardTransform();
      zooming.value = true;
      emit("zooming", true);
    } else {
      relockSessionFromOverlay();
    }

    const targetLayout = peekLayoutForClass(props.gridClass);
    applyDestPlan(zoomSession, targetLayout);

    prefetchTargetViewport(zoomSession, 24);
    await revealDestUnderneath(zoomSession, gen);
    if (gen !== zoomGen) return;
    await waitDestCoverage(gen);
    if (gen !== zoomGen) return;
    assignTileDests(zoomSession);
    zoomT = 0;
    paintFlipTiles(0);
    ensureBoardZoomLoop();
    handedOff = true;
  } catch {
    if (gen === zoomGen) finishZoom(gen);
    handedOff = true;
  } finally {
    if (!handedOff && gen === zoomGen) finishZoom(gen);
  }
}

function finishZoom(gen) {
  if (gen != null && gen !== zoomGen) return;
  stopBoardZoomLoop();
  window.clearTimeout(settleTimer);
  clearLayerZoom();
  clearZoomFades();
  clearEdgeTiles();
  clearOverlay();
  setListHidden(false);
  zoomSession = null;
  zoomScale = 1;
  zoomScaleTo = 1;
  zoomT = 0;
  zoomFade = 0;
  livePinching.value = false;
  interactionMode.value = "idle";
  suppressScrollRestore = false;
  zooming.value = false;
  emit("zooming", false);
  requestAnimationFrame(() => {
    if (gen != null && gen !== zoomGen) return;
    const needMeasure = resizeWhileZooming;
    resizeWhileZooming = false;
    if (needMeasure) measureGrid(true);
    scheduleFillCheck();
  });
}

async function commitGridZoom() {
  const session = zoomSession;
  const gen = zoomGen;
  stopBoardZoomLoop();
  window.clearTimeout(settleTimer);
  if (session) await finalizeDestToOverlay(session);
  if (gen !== zoomGen) return;
  clearOverlay();
  clearEdgeTiles();
  clearZoomFades();
  clearLayerZoom();
  setListHidden(false);
    zoomScale = 1;
    zoomScaleTo = 1;
    zoomT = 0;
    zoomFade = 0;
  zoomSession = null;
  finishZoom(gen);
}

function resetInteraction() {
  zoomGen += 1;
  pinchCommitGen += 1;
  window.clearTimeout(settleTimer);
  clearPinchWatchdog();
  stopBoardZoomLoop();
  if (pinchRaf) {
    cancelAnimationFrame(pinchRaf);
    pinchRaf = 0;
  }
  pendingPinchFrame = null;
  clearEdgeTiles();
  clearZoomFades();
  clearOverlay();
  setListHidden(false);
  clearLayerZoom();
  zoomSession = null;
  zoomScale = 1;
  zoomScaleTo = 1;
  zoomT = 0;
  zoomFade = 0;
  livePinching.value = false;
  interactionMode.value = "idle";
  zooming.value = false;
  emit("zooming", false);
  suppressScrollRestore = false;
}

function finishPinchMode() {
  resetInteraction();
  resizeWhileZooming = false;
  measureGrid(true);
}

function pinchRelPoint(session) {
  const from = session?.fromAnchor;
  if (!from || from.w < 1 || from.h < 1) return { x: 0.5, y: 0.5 };
  return {
    x: clamp((session.originX - from.x) / from.w, 0, 1),
    y: clamp((session.originY - from.y) / from.h, 0, 1),
  };
}

function indexLiveTiles(session) {
  const tiles = new Map();
  const board = overlayBoardRef.value;
  if (board) {
    for (const el of board.children) {
      const id = el.dataset.zoomId || "";
      if (id) tiles.set(id, el);
    }
  }
  session.liveTiles = tiles;
}

function ensureLiveTile(session, id) {
  let el = session.liveTiles?.get(id);
  if (el) return el;
  el = makeGhostBox({ x: 0, y: 0, w: 8, h: 8, id });
  el.style.opacity = "0";
  const src = srcForId(id);
  if (src) {
    const img = bindCoverImage(src, { fade: true, role: "new" });
    if (img) el.appendChild(img);
    beginDestLoad(src);
  }
  overlayBoardRef.value?.appendChild(el);
  session.liveTiles.set(id, el);
  return el;
}

function planForDestLayout(session, destLayout) {
  const key = `${destLayout.cols}|${destLayout.gap}|${props.items.length}`;
  const memo = session.planMemo || (session.planMemo = new Map());
  const hit = memo.get(key);
  if (hit) return hit;
  const rowW = session.rowW || measureRowWidth();
  const destPitch = latticePitch(rowW, destLayout.cols, destLayout.gap);
  const plan = planRowAnchor({
    total: props.items.length,
    fromCols: session.frozenCols,
    fromHeadRemainder: session.frozenHead,
    destCols: destLayout.cols,
    focalIndex: session.anchorIndex,
    viewportH: hostRef.value?.clientHeight || 0,
    destPitch,
    destCellH: destPitch,
    anchorRowY: session.anchorRowY,
  });
  memo.set(key, plan);
  return plan;
}

function liveLayoutMetrics(session, level, destLayout, cell, focalX, focalY) {
  const gap = Math.max(0, destLayout.gap || 0);
  const pitch = Math.max(1, cell + gap);
  const rel = session.rel || pinchRelPoint(session);
  if (level === session.startLevel) {
    const focal = itemCell(session.anchorIndex, session.frozenCols, session.frozenHead);
    return {
      cols: session.frozenCols,
      remainder: session.frozenHead,
      cell,
      pitch,
      gridX: focalX - rel.x * cell - focal.col * pitch,
      gridY: focalY - rel.y * cell - focal.row * pitch,
    };
  }
  const plan = planForDestLayout(session, destLayout);
  const focal = itemCell(session.anchorIndex, plan.destCols, plan.headRemainder);
  return {
    cols: plan.destCols,
    remainder: plan.headRemainder,
    cell,
    pitch,
    gridX: focalX - rel.x * cell - focal.col * pitch,
    gridY: focalY - rel.y * cell - focal.row * pitch,
  };
}

function collectLiveRects(metrics, hostW, hostH, into) {
  const extra = 1;
  const minRow = Math.floor((-hostH * 0.15 - metrics.gridY) / metrics.pitch) - extra;
  const maxRow = Math.ceil((hostH * 1.15 - metrics.gridY) / metrics.pitch) + extra;
  const total = props.items.length;
  for (let row = minRow; row <= maxRow; row += 1) {
    for (let col = 0; col < metrics.cols; col += 1) {
      if (into.size >= LIVE_RECT_MAX) return;
      const index = cellIndex(row, col, metrics.cols, metrics.remainder, total);
      if (index < 0) continue;
      const x = metrics.gridX + col * metrics.pitch;
      const y = metrics.gridY + row * metrics.pitch;
      if (x + metrics.cell < -metrics.cell || x > hostW + metrics.cell) continue;
      if (y + metrics.cell < -metrics.cell || y > hostH + metrics.cell) continue;
      into.set(index, { x, y, w: metrics.cell, h: metrics.cell });
    }
  }
}

function armPinchWatchdog() {
  window.clearTimeout(pinchWatchdog);
  pinchWatchdog = window.setTimeout(() => {
    pinchWatchdog = 0;
    if (interactionMode.value === "pinching" || interactionMode.value === "snapping") {
      finishPinchMode();
    }
  }, PINCH_WATCHDOG_MS);
}

function clearPinchWatchdog() {
  window.clearTimeout(pinchWatchdog);
  pinchWatchdog = 0;
}

function paintLivePinch(scale, focalX, focalY) {
  const session = zoomSession;
  if (!session) return;
  zoomScale = Math.max(0.05, Number(scale) || 1);
  session.userScale = zoomScale;
  session.focalX = focalX;
  session.focalY = focalY;

  const layouts = session.levelLayouts || readZoomLayouts();
  const maxLevel = Math.max(0, (props.zoomClasses.length || 1) - 1);
  const rowW = session.rowW || measureRowWidth();
  const brackets = pinchZoomBrackets(
    Math.max(1, session.frozenCell) * zoomScale,
    layouts,
    rowW,
    maxLevel,
  );
  if (Math.abs(zoomScale - 1) < 0.02) {
    brackets.lower = session.startLevel;
    brackets.upper = session.startLevel;
    brackets.amount = 0;
    brackets.cell = Math.max(1, session.frozenCell) * zoomScale;
  }
  const lowerLayout = layouts[brackets.lower] || { cols: session.frozenCols, gap: session.frozenGap };
  const upperLayout = layouts[brackets.upper] || lowerLayout;
  const lowerM = liveLayoutMetrics(session, brackets.lower, lowerLayout, brackets.cell, focalX, focalY);
  const upperM = liveLayoutMetrics(session, brackets.upper, upperLayout, brackets.cell, focalX, focalY);
  const hostW = hostRef.value?.clientWidth || rowW;
  const hostH = hostRef.value?.clientHeight || 0;
  const lowerRects = new Map();
  const upperRects = new Map();
  collectLiveRects(lowerM, hostW, hostH, lowerRects);
  collectLiveRects(upperM, hostW, hostH, upperRects);

  const amount = brackets.amount;
  const ids = new Set([...lowerRects.keys(), ...upperRects.keys()]);
  const seen = new Set();
  if (!session.liveTiles) indexLiveTiles(session);
  let spawned = 0;

  for (const index of ids) {
    const id = itemIdAt(index);
    if (!id) continue;
    const from = lowerRects.get(index);
    const to = upperRects.get(index);
    let rect;
    let opacity = 1;
    if (from && to) {
      rect = {
        x: lerp(from.x, to.x, amount),
        y: lerp(from.y, to.y, amount),
        w: lerp(from.w, to.w, amount),
        h: lerp(from.h, to.h, amount),
      };
    } else if (to) {
      rect = to;
      opacity = amount;
    } else {
      rect = from;
      opacity = 1 - amount;
    }
    if (opacity < 0.02) continue;
    if (!session.liveTiles.has(id)) {
      if (spawned >= LIVE_SPAWN_PER_FRAME) continue;
      spawned += 1;
    }
    const el = ensureLiveTile(session, id);
    writeGhostRect(el, rect.x, rect.y, rect.w + TILE_SEAM, rect.h + TILE_SEAM);
    el.style.opacity = String(opacity);
    seen.add(id);
  }

  for (const [id, el] of session.liveTiles) {
    if (seen.has(id)) continue;
    el.style.opacity = "0";
  }
  if (session.liveTiles.size > LIVE_TILE_MAX) {
    for (const [id, el] of session.liveTiles) {
      if (seen.has(id)) continue;
      el.remove();
      session.liveTiles.delete(id);
    }
  }

  const liveKey = `${brackets.lower}|${brackets.upper}`;
  if (session.liveKey !== liveKey) {
    session.liveKey = liveKey;
    const urls = [];
    for (const index of ids) {
      const src = props.imageSrc?.(props.items[index]);
      if (src) urls.push(src);
    }
    warmDestUrls(urls);
  }
}

function beginPinch({ focalX = 0, focalY = 0 } = {}) {
  if (props.items.length === 0) return false;
  if (interactionMode.value === "pinching") return false;
  if (interactionMode.value !== "idle" || zooming.value) resetInteraction();
  const captured = captureZoomFrom({ x: focalX, y: focalY });
  if (!captured) return false;
  captured.levelLayouts = readZoomLayouts();
  captured.previewCols = captured.frozenCols;
  captured.originX = focalX;
  captured.originY = focalY;
  captured.originLocked = true;
  captured.userScale = 1;
  captured.rel = null;
  captured.liveTiles = null;
  captured.liveKey = "";
  zoomSession = captured;
  captured.rel = pinchRelPoint(captured);
  zoomScale = 1;
  zoomScaleTo = 1;
  zoomT = 0;
  zoomFade = 0;
  mountFromOverlay(captureVisibleCards(flipLayerRef.value));
  indexLiveTiles(captured);
  resetBoardTransform();
  setListHidden(true);
  interactionMode.value = "pinching";
  livePinching.value = true;
  zooming.value = true;
  emit("zooming", true);
  suppressScrollRestore = true;
  scrollRestoreGen += 1;
  armPinchWatchdog();
  return true;
}

function updatePinch(frame) {
  if (interactionMode.value !== "pinching" || !zoomSession) return;
  pendingPinchFrame = frame;
  if (pinchRaf) return;
  pinchRaf = requestAnimationFrame(() => {
    pinchRaf = 0;
    const next = pendingPinchFrame;
    pendingPinchFrame = null;
    if (next) paintLivePinch(next.scale, next.focalX, next.focalY);
  });
}

function cancelPinch() {
  if (!zoomSession || interactionMode.value === "idle") return;
  commitPinchSnap({ targetLevel: zoomSession.startLevel });
}

async function commitPinchSnap({ targetLevel, commit } = {}) {
  if (!zoomSession || interactionMode.value !== "pinching") return;
  if (pinchRaf) {
    cancelAnimationFrame(pinchRaf);
    pinchRaf = 0;
  }
  pendingPinchFrame = null;
  clearPinchWatchdog();
  const gen = ++zoomGen;
  pinchCommitGen += 1;
  const session = zoomSession;
  const nextLevel = Number.isFinite(targetLevel)
    ? targetLevel
    : snapLevelByCellWidth({
      scale: session.userScale || zoomScale,
      startLevel: session.startLevel,
      startCellW: session.frozenCell,
      layouts: session.levelLayouts || readZoomLayouts(),
      viewportWidth: session.rowW || measureRowWidth(),
      maxLevel: Math.max(0, (props.zoomClasses.length || 1) - 1),
    });

  bakeOverlayFrom();
  if (nextLevel === session.startLevel) {
    finishPinchMode();
    return;
  }

  interactionMode.value = "snapping";
  livePinching.value = true;
  zooming.value = true;
  try {
    commit?.(nextLevel);
    await nextTick();
    if (gen !== zoomGen) return;
    const dest = peekLayoutForClass(props.gridClass);
    applyDestPlan(session, dest);
    liveGridClass.value = session.targetGridClass;
    colCount.value = session.targetCols;
    rowGapPx.value = session.targetGap;
    headRemainder.value = session.plan.headRemainder;
    listItemSize.value = Math.max(8, session.targetPitch || session.frozenPitch);
    scrollAnchorItemIndex.value = Math.max(0, session.anchorIndex);
    await nextTick();
    if (gen !== zoomGen) return;
    await scrollItemToY(session.anchorIndex, session.handoffY ?? session.anchorRowY);
  } catch {
    /* fall through to finishPinchMode */
  } finally {
    if (gen === zoomGen) finishPinchMode();
  }
}

defineExpose({
  beginPinch,
  updatePinch,
  commitPinchSnap,
  cancelPinch,
  resetInteraction,
  getHostRect,
  setZoomFocal,
  isZoomBusy,
});

watch(
  () => props.gridClass,
  (next, prev) => {
    if (interactionMode.value === "pinching" || interactionMode.value === "snapping") {
      return;
    }
    if (!prev || next === prev) {
      liveGridClass.value = next;
      return;
    }
    startGridZoom();
  },
  { flush: "pre" },
);

watch(
  () => props.items,
  (next, prev) => {
    rebuildItemIndex();
    const sameHead = !next?.length || next[0] === prev?.[0];
    if (!sameHead) headRemainder.value = 0;
    if (interactionMode.value === "idle") return;
    if (!sameHead) {
      finishPinchMode();
      return;
    }
  },
);

const rows = computed(() => {
  const photoRows = buildGridRows(props.items, colCount.value, headRemainder.value);
  const inset = Math.max(0, Math.round(Number(props.topInset) || 0));
  if (inset <= 0) return photoRows;
  return [{ kind: "inset", height: inset, startIndex: 0, items: [] }, ...photoRows];
});

function getKey(item) {
  if (typeof props.itemKey === "function") return props.itemKey(item);
  return item[props.itemKey];
}

function needsMoreItems() {
  if (interactionMode.value === "pinching") return false;
  if (props.items.length === 0) return false;
  const hostH = hostRef.value?.clientHeight || 0;
  const cols = Math.max(1, colCount.value);
  const pitch = Math.max(
    8,
    listItemSize.value || (hostRef.value?.clientWidth || 0) / cols,
  );
  if (hostH >= 8 && pitch >= 8) {
    const needed = cols * (Math.ceil(hostH / pitch) + 6);
    if (props.items.length < needed) return true;
  }
  if (zooming.value || interactionMode.value !== "idle") return false;
  if (hostH >= 8 && contentBottomPx() < hostH - 64) {
    const estimatedContent = rows.value.length * pitch;
    if (estimatedContent >= hostH + Math.min(props.bottomThreshold, 240)) {
      bumpListRelayout();
      return false;
    }
    return true;
  }
  return withVList((vl) => {
    const viewport = vl.viewportSize ?? 0;
    if (viewport < 8) return false;
    const leftover = (vl.scrollSize ?? 0) - (vl.scrollOffset ?? 0) - viewport;
    return leftover < props.bottomThreshold;
  }, false);
}

function contentBottomPx() {
  const host = hostRef.value;
  const layer = flipLayerRef.value;
  if (!host || !layer) return 0;
  const hostTop = host.getBoundingClientRect().top;
  let max = 0;
  for (const el of layer.querySelectorAll("[data-flip-id]")) {
    const bottom = el.getBoundingClientRect().bottom - hostTop;
    if (bottom > max) max = bottom;
  }
  return max;
}

function bumpListRelayout() {
  if (zooming.value || interactionMode.value !== "idle") return;
  if (relayoutAttempts >= 2) return;
  relayoutAttempts += 1;
  listEpoch.value += 1;
}

function scheduleFillCheck() {
  if (props.paused) return;
  if (fillCheckRaf) cancelAnimationFrame(fillCheckRaf);
  fillCheckRaf = requestAnimationFrame(() => {
    fillCheckRaf = 0;
    if (props.paused) return;
    nextTick(() => {
      requestAnimationFrame(() => {
        if (props.paused) return;
        if (needsMoreItems()) emit("scroll-near-bottom");
        window.clearTimeout(fillRetryTimer);
        fillRetryTimer = window.setTimeout(() => {
          if (props.paused) return;
          if (needsMoreItems()) emit("scroll-near-bottom");
        }, 180);
      });
    });
  });
}

watch(() => props.paused, (paused) => {
  if (paused) {
    if (fillCheckRaf) cancelAnimationFrame(fillCheckRaf);
    fillCheckRaf = 0;
    window.clearTimeout(fillRetryTimer);
    resetInteraction();
    return;
  }
  nextTick(() => {
    if (props.paused) return;
    measureGrid(true);
    scheduleFillCheck();
  });
});

watch(rows, scheduleFillCheck);
watch(
  () => `${colCount.value}|${headRemainder.value}|${liveGridClass.value}`,
  () => { relayoutAttempts = 0; },
);

function onScroll(offset) {
  updateScrollAnchor(offset);
  if (nearBottomPending) return;
  const nearBottom = withVList((vl) => (
    (vl.scrollSize ?? 0) - offset - (vl.viewportSize ?? 0) < props.bottomThreshold
  ), false);
  if (!nearBottom) return;
    nearBottomPending = true;
    setTimeout(() => { nearBottomPending = false; }, 300);
    emit("scroll-near-bottom");
}
</script>

<template>
  <div
    ref="hostRef"
    class="relative h-full"
    :class="[
      zooming ? 'virtual-grid-zooming overflow-hidden' : (livePinching ? 'overflow-hidden' : ''),
      paused ? 'virtual-grid-paused' : '',
    ]"
  >
    <div
      ref="probeRef"
      :class="gridClass"
      class="pointer-events-none invisible absolute left-0 top-0"
      :style="probeWidthPx > 0 ? { width: probeWidthPx + 'px' } : { width: '100%' }"
      aria-hidden="true"
    >
      <div v-for="i in 12" :key="i" />
    </div>

    <div ref="flipLayerRef" class="virtual-grid-zoom-layer absolute inset-0 overflow-visible">
    <VList
        v-if="!paused"
        :key="listLayoutKey"
      ref="listRef"
      :data="rows"
        :item-size="listItemSize"
        class="virtual-grid-scroller absolute inset-0 app-scrollbar"
        :class="zooming || livePinching ? 'pointer-events-none' : ''"
        :buffer-size="listBufferPx"
      @scroll="onScroll"
    >
      <template #default="{ item: row }">
        <div
            v-if="row.kind === 'inset'"
            aria-hidden="true"
            :style="{ height: `${row.height}px` }"
          />
          <div
            v-else
            :key="`${row.startIndex}:${colCount}:${headRemainder}`"
            :class="liveGridClass"
          :style="rowGapPx > 0 ? { paddingBottom: rowGapPx + 'px' } : undefined"
        >
          <slot
            v-for="(item, colIdx) in row.items"
            :key="getKey(item)"
            :item="item"
            :index="row.startIndex + colIdx"
          />
        </div>
      </template>
    </VList>
    </div>
    <div
      ref="ghostHostRef"
      class="virtual-grid-zoom-overlay pointer-events-none absolute inset-0 z-[2] overflow-hidden"
      aria-hidden="true"
    >
      <div ref="overlayBoardRef" class="virtual-grid-zoom-overlay-board absolute inset-0" />
    </div>
  </div>
</template>

<style scoped>
.virtual-grid-paused {
  content-visibility: hidden;
  visibility: hidden;
  pointer-events: none;
}

/* Keep the grid unpromoted at rest: iOS holds a soft raster of resting
   composited layers, and the top of the first screen then reads as blurred
   until the screen is remounted. Promote only while a zoom animation runs. */
.virtual-grid-zoom-layer {
    backface-visibility: visible;
}

.virtual-grid-zoom-overlay {
    backface-visibility: visible;
}

.virtual-grid-zoom-overlay-board {
    transform-origin: 0 0;
    backface-visibility: visible;
}

.virtual-grid-zooming .virtual-grid-zoom-layer,
.virtual-grid-zooming .virtual-grid-zoom-overlay,
.virtual-grid-zooming .virtual-grid-zoom-overlay-board {
    backface-visibility: hidden;
}

.virtual-grid-scroller {
  scrollbar-gutter: stable;
  touch-action: pan-y;
  overscroll-behavior: contain;
}

.virtual-grid-zooming :deep([data-photo-overlay]) {
  opacity: 0 !important;
  pointer-events: none;
}

.virtual-grid-zooming :deep([data-photo-below]) {
  transition: none !important;
}

.virtual-grid-zooming .virtual-grid-zoom-layer :deep([data-photo-image] img) {
  transition: none !important;
  animation: none !important;
  transform: none !important;
}
</style>
