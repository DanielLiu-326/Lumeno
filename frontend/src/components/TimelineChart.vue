<script setup>
/**
 * TimelineChart — darktable-style timeline bar chart.
 *
 * Zoom levels (Ctrl + Wheel):
 *   year    – each block = 1 year,    bars = months
 *   quarter – each block = 3 months,  bars = days
 *   month   – each block = 1 month,   bars = days
 *
 * Native scrollbar for horizontal panning (app-scrollbar style).
 * Click-drag to select a date range; selection edges can be resized.
 * Right-click clears the selection. Defaults to showing the latest dates.
 */
import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick } from "vue";
import { t } from "../i18n/index.js";

const props = defineProps({
  buckets: { type: Array, default: () => [] },
  isDark: { type: Boolean, default: false },
  from: { type: String, default: "" },
  to: { type: String, default: "" },
});

const emit = defineEmits(["select-range"]);

// ── Refs ──────────────────────────────────────────────────────────────────
const wrapperRef = ref(null);
const canvasRef = ref(null);
const viewWidth = ref(400);
const scrollPx = ref(0); // current scrollLeft of wrapper

// ── Constants ─────────────────────────────────────────────────────────────
const BLOCK_GAP = 3;
const LABEL_H = 14;
const PAD_T = 2;

// ── Zoom table ────────────────────────────────────────────────────────────
// [blockType, barWidth]. "year" bars=months, "quarter" bars=days, "month" bars=days.
const ZOOM_LEVELS = [
  ["year",     1],  // 0:  ultra-compact years
  ["year",     2],  // 1:  very compact years
  ["year",     4],  // 2:  compact years
  ["year",     7],  // 3:  medium years
  ["year",    12],  // 4:  wide years
  ["year",    20],  // 5:  extra-wide years
  ["quarter",  1],  // 6:  compact quarters (days)
  ["quarter",  2],  // 7:  medium quarters
  ["quarter",  3],  // 8:  wide quarters
  ["month",    2],  // 9:  very compact days
  ["month",    3],  // 10: compact days
  ["month",    5],  // 11: medium days
  ["month",    8],  // 12: wide days
  ["month",   12],  // 13: extra-wide days
];

// ── State ─────────────────────────────────────────────────────────────────
const zoomIdx = ref(2);
const zoomLevel = computed(() => ZOOM_LEVELS[zoomIdx.value][0]);

const mouseX = ref(-1);
const mouseIn = ref(false);

const selecting = ref(false);
const selStartX = ref(-1);
const selEndX = ref(-1);

// ── Pre-process data ──────────────────────────────────────────────────────
function daysInMonth(y, m) {
  return new Date(y, m, 0).getDate();
}

const dayMap = computed(() => {
  const m = new Map();
  for (const b of props.buckets) m.set(b.date, b.count);
  return m;
});

const monthMap = computed(() => {
  const m = new Map();
  for (const b of props.buckets) {
    const k = b.date.slice(0, 7);
    m.set(k, (m.get(k) || 0) + b.count);
  }
  return m;
});

const timeRange = computed(() => {
  const bkts = props.buckets;
  if (bkts.length === 0) return null;
  const first = bkts[0].date;
  const last = bkts[bkts.length - 1].date;
  const [y0, m0] = first.split("-").map(Number);
  const [y1, m1] = last.split("-").map(Number);
  return { y0, m0, y1, m1 };
});

// ── Block definitions ─────────────────────────────────────────────────────
const blocks = computed(() => {
  const tr = timeRange.value;
  if (!tr) return [];
  const z = zoomLevel.value;
  const out = [];

  if (z === "year") {
    for (let y = tr.y0; y <= tr.y1; y++) {
      const bars = [];
      for (let m = 1; m <= 12; m++) {
        const key = `${y}-${String(m).padStart(2, "0")}`;
        bars.push({ key, count: monthMap.value.get(key) || 0 });
      }
      out.push({ label: String(y), bars, dateFrom: `${y}-01-01`, dateTo: `${y}-12-31` });
    }
  } else if (z === "quarter") {
    let y = tr.y0, m = tr.m0;
    m = m - ((m - 1) % 3);
    while (y < tr.y1 || (y === tr.y1 && m <= tr.m1)) {
      const bars = [];
      const qEnd = Math.min(m + 2, 12);
      const actualEnd = (y === tr.y1) ? Math.min(qEnd, tr.m1) : qEnd;
      for (let cm = m; cm <= actualEnd; cm++) {
        const ym = `${y}-${String(cm).padStart(2, "0")}`;
        const dim = daysInMonth(y, cm);
        for (let d = 1; d <= dim; d++) {
          const key = `${ym}-${String(d).padStart(2, "0")}`;
          bars.push({ key, count: dayMap.value.get(key) || 0 });
        }
      }
      const lastM = actualEnd;
      const dimLast = daysInMonth(y, lastM);
      out.push({
        label: `${y}.${String(m).padStart(2, "0")}-${String(lastM).padStart(2, "0")}`,
        bars,
        dateFrom: `${y}-${String(m).padStart(2, "0")}-01`,
        dateTo: `${y}-${String(lastM).padStart(2, "0")}-${String(dimLast).padStart(2, "0")}`,
      });
      m += 3;
      if (m > 12) { m = 1; y++; }
    }
  } else {
    let y = tr.y0, m = tr.m0;
    while (y < tr.y1 || (y === tr.y1 && m <= tr.m1)) {
      const ym = `${y}-${String(m).padStart(2, "0")}`;
      const dim = daysInMonth(y, m);
      const bars = [];
      for (let d = 1; d <= dim; d++) {
        const key = `${ym}-${String(d).padStart(2, "0")}`;
        bars.push({ key, count: dayMap.value.get(key) || 0 });
      }
      out.push({
        label: `${y}.${String(m).padStart(2, "0")}`,
        bars,
        dateFrom: `${ym}-01`,
        dateTo: `${ym}-${String(dim).padStart(2, "0")}`,
      });
      m++;
      if (m > 12) { m = 1; y++; }
    }
  }
  return out;
});

const barW = computed(() => ZOOM_LEVELS[zoomIdx.value][1]);

function blockWidth(block) {
  return block.bars.length * barW.value;
}

// Total content width in pixels.
const stripWidth = computed(() => {
  const bl = blocks.value;
  if (bl.length === 0) return 0;
  let w = 0;
  for (const b of bl) w += blockWidth(b);
  w += (bl.length - 1) * BLOCK_GAP;
  return w;
});

// ── Pixel helpers (absolute strip coords) ─────────────────────────────────
// Absolute X of block start from strip origin.
function blockAbsX(blockIdx) {
  const bl = blocks.value;
  let x = 0;
  for (let i = 0; i < blockIdx; i++) x += blockWidth(bl[i]) + BLOCK_GAP;
  return x;
}

// Absolute X of a date in the strip.
function dateAbsX(date) {
  if (!date) return -1;
  const bl = blocks.value;
  const bw = barW.value;
  for (let bi = 0; bi < bl.length; bi++) {
    for (let i = 0; i < bl[bi].bars.length; i++) {
      if (bl[bi].bars[i].key === date) {
        return blockAbsX(bi) + i * bw + bw / 2;
      }
    }
    if (bl[bi].dateTo >= date) {
      // Closest bar
      for (let i = 0; i < bl[bi].bars.length; i++) {
        if (bl[bi].bars[i].key >= date) return blockAbsX(bi) + i * bw + bw / 2;
      }
      return blockAbsX(bi) + blockWidth(bl[bi]) - bw / 2;
    }
  }
  return stripWidth.value;
}

// Convert canvas x → absolute strip x.
function canvasToAbs(cx) { return cx + scrollPx.value; }
// Convert absolute strip x → canvas x.
function absToCanvas(ax) { return ax - scrollPx.value; }

// ── Bar-height function (darktable: sqrt scaling) ─────────────────────────
function barHeight(count, maxH) {
  if (count === 0) return 0;
  return maxH * (1.0 - 2.0 / Math.sqrt(count + 4.0));
}

// ── Pixel ↔ date mapping ──────────────────────────────────────────────────
function hitTest(canvasX) {
  const absX = canvasToAbs(canvasX);
  const bl = blocks.value;
  const bw = barW.value;
  let px = 0;

  for (let bi = 0; bi < bl.length; bi++) {
    const bWidth = blockWidth(bl[bi]);
    if (absX >= px && absX < px + bWidth) {
      const barIdx = Math.floor((absX - px) / bw);
      return { bi, barIdx: Math.min(barIdx, bl[bi].bars.length - 1) };
    }
    px += bWidth + BLOCK_GAP;
  }
  if (bl.length > 0) {
    const lastBi = bl.length - 1;
    return { bi: lastBi, barIdx: bl[lastBi].bars.length - 1 };
  }
  return null;
}

function hitToDate(canvasX) {
  const hit = hitTest(canvasX);
  if (!hit) return null;
  const bl = blocks.value[hit.bi];
  if (!bl) return null;
  return bl.bars[hit.barIdx]?.key || null;
}

// ── Selection → date range ────────────────────────────────────────────────
function selectionDates() {
  if (selStartX.value < 0 || selEndX.value < 0) return null;
  const d1 = hitToDate(Math.min(selStartX.value, selEndX.value));
  const d2 = hitToDate(Math.max(selStartX.value, selEndX.value));
  if (!d1 || !d2) return null;

  let from, to;
  if (zoomLevel.value === "year") {
    from = d1 + "-01";
    const [y2, m2] = d2.split("-").map(Number);
    to = d2 + "-" + String(daysInMonth(y2, m2)).padStart(2, "0");
  } else {
    from = d1;
    to = d2;
  }
  return { from, to };
}

// ── Canvas drawing ────────────────────────────────────────────────────────
function draw() {
  const canvas = canvasRef.value;
  if (!canvas) return;

  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (w === 0 || h === 0) return;

  canvas.width = w * dpr;
  canvas.height = h * dpr;
  const ctx = canvas.getContext("2d");
  ctx.scale(dpr, dpr);

  const isDark = props.isDark;
  const chartH = h - LABEL_H - PAD_T;
  const bl = blocks.value;
  const sp = scrollPx.value;

  // Background
  ctx.fillStyle = isDark ? "#1c1917" : "#fafaf9";
  ctx.fillRect(0, 0, w, h);

  if (bl.length === 0) {
    ctx.fillStyle = isDark ? "#57534e" : "#a8a29e";
    ctx.font = "10px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.fillText(t("albums.noPhotos") || "No data", w / 2, h / 2);
    return;
  }

  let globalMax = 1;
  for (const b of bl) {
    for (const bar of b.bars) {
      if (bar.count > globalMax) globalMax = bar.count;
    }
  }

  const bw = barW.value;
  const maxBarH = chartH - 2;

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, w, h);
  ctx.clip();

  const propSel = getSelectionPixels(w);
  let absPx = 0;

  for (let bi = 0; bi < bl.length; bi++) {
    const block = bl[bi];
    const bWidth = blockWidth(block);
    const screenX = absPx - sp;

    // Skip off-screen blocks
    if (screenX + bWidth < 0) { absPx += bWidth + BLOCK_GAP; continue; }
    if (screenX > w) break;

    // Block background
    ctx.fillStyle = isDark ? "#292524" : "#f5f5f4";
    ctx.fillRect(screenX, PAD_T, bWidth, chartH);

    // Bars
    for (let i = 0; i < block.bars.length; i++) {
      const bar = block.bars[i];
      if (bar.count === 0) continue;
      const bh = barHeight(bar.count, maxBarH);
      const x = screenX + i * bw;
      const y = PAD_T + chartH - bh;

      const inSel = isDateInSelection(bar.key);
      const inDrag = selecting.value && isDragSelected(x, bw);

      if (inSel || inDrag) {
        ctx.fillStyle = isDark ? "#a78bfa" : "#6d28d9";
      } else {
        ctx.fillStyle = isDark ? "rgba(168,162,158,0.55)" : "rgba(120,113,108,0.55)";
      }
      ctx.fillRect(x, y, bw - (bw > 3 ? 1 : 0), bh);
    }

    // Block label
    ctx.fillStyle = isDark ? "#a8a29e" : "#78716c";
    ctx.font = "9px system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.fillText(block.label, screenX + bWidth / 2, h - 2);

    absPx += bWidth + BLOCK_GAP;
  }

  // Selection overlay
  const selPx = selecting.value ? getDragPixels() : propSel;
  if (selPx) {
    const { x1, x2 } = selPx;
    ctx.fillStyle = isDark ? "rgba(139,92,246,0.18)" : "rgba(124,58,237,0.12)";
    ctx.fillRect(x1, 0, x2 - x1, h);
    ctx.strokeStyle = isDark ? "rgba(167,139,250,0.7)" : "rgba(109,40,217,0.6)";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(x1, 0); ctx.lineTo(x1, h);
    ctx.moveTo(x2, 0); ctx.lineTo(x2, h);
    ctx.stroke();
  }

  // Cursor line + tooltip
  if (mouseIn.value && mouseX.value >= 0 && !selecting.value) {
    const date = hitToDate(mouseX.value);
    if (date) {
      ctx.strokeStyle = isDark ? "rgba(231,229,228,0.35)" : "rgba(28,25,23,0.25)";
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(mouseX.value, 0);
      ctx.lineTo(mouseX.value, h);
      ctx.stroke();
      ctx.setLineDash([]);

      const hit = hitTest(mouseX.value);
      let text = date;
      if (hit) {
        const bar = bl[hit.bi]?.bars[hit.barIdx];
        if (bar && bar.count > 0) text += `  ${bar.count}`;
      }

      ctx.font = "10px system-ui, sans-serif";
      const tw = ctx.measureText(text).width;
      const tx = Math.min(mouseX.value + 6, w - tw - 10);
      const ty = 6;
      ctx.fillStyle = isDark ? "rgba(41,37,36,0.92)" : "rgba(250,250,249,0.92)";
      const rr = 3;
      roundRect(ctx, tx, ty, tw + 10, 18, rr);
      ctx.fill();
      ctx.strokeStyle = isDark ? "#57534e" : "#d6d3d1";
      ctx.lineWidth = 0.5;
      roundRect(ctx, tx, ty, tw + 10, 18, rr);
      ctx.stroke();
      ctx.fillStyle = isDark ? "#e7e5e4" : "#1c1917";
      ctx.textAlign = "left";
      ctx.fillText(text, tx + 5, ty + 13);
    }
  }

  ctx.restore();
}

function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.lineTo(x + w - r, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + r);
  ctx.lineTo(x + w, y + h - r);
  ctx.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
  ctx.lineTo(x + r, y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - r);
  ctx.lineTo(x, y + r);
  ctx.quadraticCurveTo(x, y, x + r, y);
  ctx.closePath();
}

function isDateInSelection(barKey) {
  const f = props.from;
  const tt = props.to;
  if (!f && !tt) return false;

  if (zoomLevel.value === "year") {
    const barFrom = barKey + "-01";
    const [y, m] = barKey.split("-").map(Number);
    const barTo = barKey + "-" + String(daysInMonth(y, m)).padStart(2, "0");
    return (!f || barTo >= f) && (!tt || barFrom <= tt);
  } else {
    return (!f || barKey >= f) && (!tt || barKey <= tt);
  }
}

function isDragSelected(barX, bw) {
  if (!selecting.value) return false;
  const x1 = Math.min(selStartX.value, selEndX.value);
  const x2 = Math.max(selStartX.value, selEndX.value);
  return barX + bw > x1 && barX < x2;
}

function getDragPixels() {
  if (selStartX.value < 0) return null;
  return {
    x1: Math.min(selStartX.value, selEndX.value),
    x2: Math.max(selStartX.value, selEndX.value),
  };
}

function getSelectionPixels(canvasW) {
  if (!props.from && !props.to) return null;
  const x1 = findPixelForDate(props.from, true);
  const x2 = findPixelForDate(props.to, false);
  if (x1 < 0 && x2 < 0) return null;
  return { x1: Math.max(0, x1), x2: Math.min(canvasW, x2) };
}

function findPixelForDate(date, isStart) {
  if (!date) return isStart ? 0 : viewWidth.value;
  const bl = blocks.value;
  const bw = barW.value;
  const sp = scrollPx.value;

  let absPx = 0;
  for (let bi = 0; bi < bl.length; bi++) {
    for (let i = 0; i < bl[bi].bars.length; i++) {
      const barDate = bl[bi].bars[i].key;
      let barFrom, barTo;
      if (zoomLevel.value === "year") {
        barFrom = barDate + "-01";
        const [y, m] = barDate.split("-").map(Number);
        barTo = barDate + "-" + String(daysInMonth(y, m)).padStart(2, "0");
      } else {
        barFrom = barDate;
        barTo = barDate;
      }
      const screenX = absPx + i * bw - sp;
      if (isStart && barTo >= date) return screenX;
      if (!isStart && barFrom > date) return screenX;
    }
    absPx += blockWidth(bl[bi]) + BLOCK_GAP;
  }
  return isStart ? 0 : viewWidth.value;
}

// ── Mouse handlers ────────────────────────────────────────────────────────
const EDGE_THRESHOLD = 6;

function getCanvasX(e) {
  const rect = canvasRef.value?.getBoundingClientRect();
  return rect ? e.clientX - rect.left : 0;
}

function onMouseDown(e) {
  if (e.button === 2) {
    e.preventDefault();
    emit("select-range", "", "");
    return;
  }
  if (e.button !== 0) return;
  const x = getCanvasX(e);

  const sel = getSelectionPixels(viewWidth.value);
  if (sel) {
    if (Math.abs(x - sel.x1) <= EDGE_THRESHOLD) {
      selStartX.value = sel.x2;
      selEndX.value = x;
      selecting.value = true;
      e.preventDefault();
      return;
    }
    if (Math.abs(x - sel.x2) <= EDGE_THRESHOLD) {
      selStartX.value = sel.x1;
      selEndX.value = x;
      selecting.value = true;
      e.preventDefault();
      return;
    }
  }

  selStartX.value = x;
  selEndX.value = x;
  selecting.value = true;
  e.preventDefault();
}

function onMouseMove(e) {
  const x = getCanvasX(e);
  mouseX.value = x;
  mouseIn.value = true;

  if (selecting.value) {
    selEndX.value = x;
  }

  if (!selecting.value && canvasRef.value) {
    const sel = getSelectionPixels(viewWidth.value);
    if (sel && (Math.abs(x - sel.x1) <= EDGE_THRESHOLD || Math.abs(x - sel.x2) <= EDGE_THRESHOLD)) {
      canvasRef.value.style.cursor = "col-resize";
    } else {
      canvasRef.value.style.cursor = "crosshair";
    }
  }

  requestDraw();
}

function onMouseUp(e) {
  if (!selecting.value) return;
  selEndX.value = getCanvasX(e);
  selecting.value = false;

  if (Math.abs(selEndX.value - selStartX.value) < 3) {
    emit("select-range", "", "");
  } else {
    const dates = selectionDates();
    if (dates) emit("select-range", dates.from, dates.to);
  }
  selStartX.value = -1;
  selEndX.value = -1;
  requestDraw();
}

function onMouseLeave() {
  mouseIn.value = false;
  if (selecting.value) {
    selecting.value = false;
    const dates = selectionDates();
    if (dates) emit("select-range", dates.from, dates.to);
    selStartX.value = -1;
    selEndX.value = -1;
  }
  requestDraw();
}

function onContextMenu(e) {
  e.preventDefault();
}

// ── Wheel: Ctrl+wheel = zoom, plain wheel = horizontal pan ────────────────
function onWheel(e) {
  e.preventDefault();

  if (e.ctrlKey || e.metaKey) {
    const mx = mouseX.value >= 0 ? mouseX.value : viewWidth.value / 2;
    const date = hitToDate(mx);
    setZoomIndex(zoomIdx.value + (e.deltaY < 0 ? 1 : -1), date, mx);
  } else {
    // Translate vertical wheel into horizontal scroll
    const delta = e.deltaY !== 0 ? e.deltaY : e.deltaX;
    setScrollLeft(scrollPx.value + delta);
  }
}

// ── Scroll sync ───────────────────────────────────────────────────────────
function onScroll() {
  if (wrapperRef.value) {
    scrollPx.value = wrapperRef.value.scrollLeft;
    requestDraw();
  }
}

function setScrollLeft(px) {
  if (wrapperRef.value) {
    wrapperRef.value.scrollLeft = Math.max(0, Math.min(px, stripWidth.value - viewWidth.value));
    scrollPx.value = wrapperRef.value.scrollLeft;
  }
}

function scrollToEnd() {
  nextTick(() => setScrollLeft(stripWidth.value));
}

function scrollToDateAtX(date, targetX) {
  if (!date) return;
  const absPx = dateAbsX(date);
  if (absPx < 0) return;
  setScrollLeft(absPx - targetX);
}

function setZoomIndex(index, date, targetX) {
  const newIdx = Math.max(0, Math.min(index, ZOOM_LEVELS.length - 1));
  if (newIdx === zoomIdx.value) return;
  zoomIdx.value = newIdx;
  nextTick(() => { scrollToDateAtX(date, targetX); requestDraw(); });
}

// ── Touch handlers ────────────────────────────────────────────────────────
let pinchStartDistance = 0;
let pinchStartZoom = 0;
let pinchDate = null;

function touchDistance(touches) {
  return Math.hypot(
    touches[0].clientX - touches[1].clientX,
    touches[0].clientY - touches[1].clientY,
  );
}

function touchCenterX(touches, rect) {
  return (touches[0].clientX + touches[1].clientX) / 2 - rect.left;
}

function onTouchStart(e) {
  if (e.touches.length === 2) {
    e.preventDefault();
    const rect = canvasRef.value.getBoundingClientRect();
    const centerX = touchCenterX(e.touches, rect);
    pinchStartDistance = touchDistance(e.touches);
    pinchStartZoom = zoomIdx.value;
    pinchDate = hitToDate(centerX);
    selecting.value = false;
    selStartX.value = -1;
    selEndX.value = -1;
    return;
  }
  if (e.touches.length !== 1 || pinchStartDistance > 0) return;
  const rect = canvasRef.value.getBoundingClientRect();
  const x = e.touches[0].clientX - rect.left;
  selStartX.value = x;
  selEndX.value = x;
  selecting.value = true;
}

function onTouchMove(e) {
  if (e.touches.length === 2 && pinchStartDistance > 0) {
    e.preventDefault();
    const scale = touchDistance(e.touches) / pinchStartDistance;
    const indexDelta = Math.round(Math.log2(scale) * 4);
    const rect = canvasRef.value.getBoundingClientRect();
    setZoomIndex(pinchStartZoom + indexDelta, pinchDate, touchCenterX(e.touches, rect));
    return;
  }
  if (!selecting.value || e.touches.length !== 1) return;
  const rect = canvasRef.value.getBoundingClientRect();
  selEndX.value = e.touches[0].clientX - rect.left;
  e.preventDefault();
  requestDraw();
}

function onTouchEnd(e) {
  if (pinchStartDistance > 0) {
    if (e.touches.length < 2) {
      pinchStartDistance = 0;
      pinchDate = null;
    }
    return;
  }
  if (!selecting.value) return;
  selecting.value = false;
  if (Math.abs(selEndX.value - selStartX.value) < 3) {
    emit("select-range", "", "");
  } else {
    const dates = selectionDates();
    if (dates) emit("select-range", dates.from, dates.to);
  }
  selStartX.value = -1;
  selEndX.value = -1;
  requestDraw();
}

// ── Animation frame for drawing ───────────────────────────────────────────
let drawRaf = 0;
function requestDraw() {
  if (drawRaf) return;
  drawRaf = requestAnimationFrame(() => { drawRaf = 0; draw(); });
}

// ── Resize observer ───────────────────────────────────────────────────────
let resizeObs = null;
function measure() {
  if (wrapperRef.value) {
    viewWidth.value = wrapperRef.value.clientWidth;
    requestDraw();
  }
}

watch(() => props.buckets, () => {
  zoomIdx.value = 2;
  nextTick(() => { scrollToEnd(); requestDraw(); });
});

watch([() => props.from, () => props.to, () => props.isDark], () => requestDraw());

onMounted(() => {
  measure();
  resizeObs = new ResizeObserver(measure);
  if (wrapperRef.value) resizeObs.observe(wrapperRef.value);
  // Default: show latest dates (scroll to end)
  nextTick(scrollToEnd);
});

onBeforeUnmount(() => {
  if (resizeObs) resizeObs.disconnect();
  if (drawRaf) cancelAnimationFrame(drawRaf);
});
</script>

<template>
  <div
    ref="wrapperRef"
    class="app-scrollbar w-full overflow-x-auto"
    @scroll.passive="onScroll"
  >
    <div :style="{ width: Math.max(stripWidth, viewWidth) + 'px', position: 'relative' }">
      <canvas
        ref="canvasRef"
        class="block select-none sticky left-0"
        :style="{ width: viewWidth + 'px', height: '96px', touchAction: 'pan-y' }"
        @mousedown="onMouseDown"
        @mousemove="onMouseMove"
        @mouseup="onMouseUp"
        @mouseleave="onMouseLeave"
        @wheel.prevent="onWheel"
        @contextmenu="onContextMenu"
        @touchstart="onTouchStart"
        @touchmove="onTouchMove"
        @touchend="onTouchEnd"
        @touchcancel="onTouchEnd"
      />
    </div>
  </div>
</template>
