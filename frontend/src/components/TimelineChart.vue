<script setup>
/**
 * TimelineChart — darktable-style timeline bar chart.
 *
 * Zoom levels (Ctrl + Wheel):
 *   year    – each block = 1 year,    bars = months
 *   quarter – each block = 3 months,  bars = days
 *   month   – each block = 1 month,   bars = days
 *
 * Native scrollbar for panning (horizontal on desktop, vertical on mobile).
 * Click-drag to select a date range; selection edges can be resized.
 * Right-click clears the selection. Defaults to showing the latest dates.
 *
 * Mobile (`vertical`): time runs top → bottom, each count bar grows left → right.
 */
import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick } from "vue";
import { t } from "../i18n/index.js";
import { isMobileShell } from "../utils/device.js";

const props = defineProps({
  buckets: { type: Array, default: () => [] },
  isDark: { type: Boolean, default: false },
  from: { type: String, default: "" },
  to: { type: String, default: "" },
  vertical: { type: Boolean, default: () => isMobileShell() },
});

const emit = defineEmits(["select-range"]);

const BLOCK_GAP = 3;
const LABEL_H = 14;
const LABEL_W = 52;
const PAD_T = 2;
const PAD_L = 2;
const VERTICAL_H = 360;

// ── Refs ──────────────────────────────────────────────────────────────────
const wrapperRef = ref(null);
const canvasRef = ref(null);
const viewWidth = ref(400);
const viewHeight = ref(props.vertical ? VERTICAL_H : 96);
const scrollPx = ref(0); // scroll along the time axis

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
const zoomIdx = ref(props.vertical ? 3 : 2);
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

function viewExtent() {
  return props.vertical ? viewHeight.value : viewWidth.value;
}

function edgeThreshold() {
  return props.vertical ? 12 : 6;
}

// Total content length along the time axis.
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
function canvasToAbs(cx) {
  return props.vertical ? cx : cx + scrollPx.value;
}
function absToCanvas(ax) {
  return props.vertical ? ax : ax - scrollPx.value;
}

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
  const vertical = props.vertical;
  const chartH = h - LABEL_H - PAD_T;
  const chartW = w - LABEL_W - PAD_L;
  const bl = blocks.value;
  const sp = props.vertical ? 0 : scrollPx.value;
  const viewLen = vertical ? h : w;

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

  const bw = barW.value;
  const maxBarLen = (vertical ? chartW : chartH) - 2;
  const gap = bw > 3 ? 1 : 0;

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, w, h);
  ctx.clip();

  const propSel = getSelectionPixels();
  let absPx = 0;

  for (let bi = 0; bi < bl.length; bi++) {
    const block = bl[bi];
    const bLen = blockWidth(block);
    const screenPos = absPx - sp;

    if (screenPos + bLen < 0) { absPx += bLen + BLOCK_GAP; continue; }
    if (screenPos > viewLen) break;

    ctx.fillStyle = isDark ? "#292524" : "#f5f5f4";
    if (vertical) {
      ctx.fillRect(LABEL_W, screenPos, chartW, bLen);
    } else {
      ctx.fillRect(screenPos, PAD_T, bLen, chartH);
    }

    for (let i = 0; i < block.bars.length; i++) {
      const bar = block.bars[i];
      if (bar.count === 0) continue;
      const blen = barHeight(bar.count, maxBarLen);
      let barPos;
      if (vertical) {
        const y = screenPos + i * bw;
        barPos = y;
        const inSel = isDateInSelection(bar.key);
        const inDrag = selecting.value && isDragSelected(barPos, bw);
        ctx.fillStyle = (inSel || inDrag)
          ? (isDark ? "#a78bfa" : "#6d28d9")
          : (isDark ? "rgba(168,162,158,0.55)" : "rgba(120,113,108,0.55)");
        ctx.fillRect(LABEL_W, y, blen, bw - gap);
      } else {
        const x = screenPos + i * bw;
        barPos = x;
        const y = PAD_T + chartH - blen;
        const inSel = isDateInSelection(bar.key);
        const inDrag = selecting.value && isDragSelected(barPos, bw);
        ctx.fillStyle = (inSel || inDrag)
          ? (isDark ? "#a78bfa" : "#6d28d9")
          : (isDark ? "rgba(168,162,158,0.55)" : "rgba(120,113,108,0.55)");
        ctx.fillRect(x, y, bw - gap, blen);
      }
    }

    ctx.fillStyle = isDark ? "#a8a29e" : "#78716c";
    ctx.font = "9px system-ui, sans-serif";
    ctx.textAlign = "center";
    if (vertical) {
      if (bLen >= 11) {
        ctx.textBaseline = "middle";
        ctx.fillText(block.label, LABEL_W / 2, screenPos + bLen / 2, LABEL_W - 4);
        ctx.textBaseline = "alphabetic";
      }
    } else {
      ctx.fillText(block.label, screenPos + bLen / 2, h - 2);
    }

    absPx += bLen + BLOCK_GAP;
  }

  // Selection overlay
  const selPx = selecting.value ? getDragPixels() : propSel;
  if (selPx) {
    const { x1, x2 } = selPx;
    ctx.fillStyle = isDark ? "rgba(139,92,246,0.18)" : "rgba(124,58,237,0.12)";
    ctx.strokeStyle = isDark ? "rgba(167,139,250,0.7)" : "rgba(109,40,217,0.6)";
    ctx.lineWidth = 1.5;
    if (vertical) {
      ctx.fillRect(0, x1, w, x2 - x1);
      ctx.beginPath();
      ctx.moveTo(0, x1); ctx.lineTo(w, x1);
      ctx.moveTo(0, x2); ctx.lineTo(w, x2);
      ctx.stroke();
    } else {
      ctx.fillRect(x1, 0, x2 - x1, h);
      ctx.beginPath();
      ctx.moveTo(x1, 0); ctx.lineTo(x1, h);
      ctx.moveTo(x2, 0); ctx.lineTo(x2, h);
      ctx.stroke();
    }
  }

  // Cursor line + tooltip
  if (mouseIn.value && mouseX.value >= 0 && !selecting.value) {
    const date = hitToDate(mouseX.value);
    if (date) {
      ctx.strokeStyle = isDark ? "rgba(231,229,228,0.35)" : "rgba(28,25,23,0.25)";
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      if (vertical) {
        ctx.moveTo(0, mouseX.value);
        ctx.lineTo(w, mouseX.value);
      } else {
        ctx.moveTo(mouseX.value, 0);
        ctx.lineTo(mouseX.value, h);
      }
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
      let tx, ty;
      if (vertical) {
        tx = Math.min(LABEL_W + 8, w - tw - 10);
        ty = Math.min(mouseX.value + 8, h - 22);
      } else {
        tx = Math.min(mouseX.value + 6, w - tw - 10);
        ty = 6;
      }
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

function getSelectionPixels() {
  if (!props.from && !props.to) return null;
  const x1 = findPixelForDate(props.from, true);
  const x2 = findPixelForDate(props.to, false);
  if (x1 < 0 && x2 < 0) return null;
  const len = props.vertical ? Math.max(stripWidth.value, viewHeight.value) : viewWidth.value;
  return { x1: Math.max(0, x1), x2: Math.min(len, x2) };
}

function findPixelForDate(date, isStart) {
  if (!date) return isStart ? 0 : (props.vertical ? stripWidth.value : viewExtent());
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
      const screenX = absPx + i * bw - (props.vertical ? 0 : sp);
      if (isStart && barTo >= date) return screenX;
      if (!isStart && barFrom > date) return screenX;
    }
    absPx += blockWidth(bl[bi]) + BLOCK_GAP;
  }
  return isStart ? 0 : (props.vertical ? stripWidth.value : viewExtent());
}

// ── Mouse handlers ────────────────────────────────────────────────────────
function getCanvasPos(e) {
  const rect = canvasRef.value?.getBoundingClientRect();
  if (!rect) return 0;
  return props.vertical ? e.clientY - rect.top : e.clientX - rect.left;
}

function onMouseDown(e) {
  if (e.button === 2) {
    e.preventDefault();
    emit("select-range", "", "");
    return;
  }
  if (e.button !== 0) return;
  const x = getCanvasPos(e);
  const edge = edgeThreshold();

  const sel = getSelectionPixels();
  if (sel) {
    if (Math.abs(x - sel.x1) <= edge) {
      selStartX.value = sel.x2;
      selEndX.value = x;
      selecting.value = true;
      e.preventDefault();
      return;
    }
    if (Math.abs(x - sel.x2) <= edge) {
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
  const x = getCanvasPos(e);
  mouseX.value = x;
  mouseIn.value = true;

  if (selecting.value) {
    selEndX.value = x;
  }

  if (!selecting.value && canvasRef.value) {
    const sel = getSelectionPixels();
    const edge = edgeThreshold();
    if (sel && (Math.abs(x - sel.x1) <= edge || Math.abs(x - sel.x2) <= edge)) {
      canvasRef.value.style.cursor = props.vertical ? "row-resize" : "col-resize";
    } else {
      canvasRef.value.style.cursor = "crosshair";
    }
  }

  requestDraw();
}

function onMouseUp(e) {
  if (!selecting.value) return;
  selEndX.value = getCanvasPos(e);
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

// ── Wheel: Ctrl+wheel = zoom; otherwise pan (native when vertical) ────────
function viewportAxisPos(clientX, clientY) {
  const wrap = wrapperRef.value?.getBoundingClientRect();
  if (!wrap) return 0;
  return props.vertical ? clientY - wrap.top : clientX - wrap.left;
}

function onWheel(e) {
  if (e.ctrlKey || e.metaKey) {
    e.preventDefault();
    const date = hitToDate(getCanvasPos(e));
    setZoomIndex(
      zoomIdx.value + (e.deltaY < 0 ? 1 : -1),
      date,
      viewportAxisPos(e.clientX, e.clientY),
    );
    return;
  }
  if (props.vertical) return;
  e.preventDefault();
  const delta = e.deltaY !== 0 ? e.deltaY : e.deltaX;
  setScrollLeft(scrollPx.value + delta);
}

// ── Scroll sync ───────────────────────────────────────────────────────────
function onScroll() {
  if (!wrapperRef.value) return;
  scrollPx.value = props.vertical ? wrapperRef.value.scrollTop : wrapperRef.value.scrollLeft;
  requestDraw();
}

function setScrollLeft(px) {
  if (!wrapperRef.value) return;
  const max = Math.max(0, stripWidth.value - viewExtent());
  const next = Math.max(0, Math.min(px, max));
  if (props.vertical) wrapperRef.value.scrollTop = next;
  else wrapperRef.value.scrollLeft = next;
  scrollPx.value = props.vertical ? wrapperRef.value.scrollTop : wrapperRef.value.scrollLeft;
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
let pressTimer = 0;
let pressPos = 0;
let pressClientX = 0;
let pressClientY = 0;

function clearPressTimer() {
  if (pressTimer) {
    clearTimeout(pressTimer);
    pressTimer = 0;
  }
}

function touchDistance(touches) {
  return Math.hypot(
    touches[0].clientX - touches[1].clientX,
    touches[0].clientY - touches[1].clientY,
  );
}

function touchCenterPos(touches, rect) {
  if (props.vertical) {
    return (touches[0].clientY + touches[1].clientY) / 2 - rect.top;
  }
  return (touches[0].clientX + touches[1].clientX) / 2 - rect.left;
}

function touchAxisPos(touch, rect) {
  return props.vertical ? touch.clientY - rect.top : touch.clientX - rect.left;
}

function emitTapOrRange() {
  if (Math.abs(selEndX.value - selStartX.value) < 3) {
    const date = hitToDate(selStartX.value);
    if (props.vertical && date) {
      selEndX.value = selStartX.value;
      const dates = selectionDates();
      if (dates) emit("select-range", dates.from, dates.to);
      else emit("select-range", "", "");
    } else {
      emit("select-range", "", "");
    }
  } else {
    const dates = selectionDates();
    if (dates) emit("select-range", dates.from, dates.to);
  }
}

function onTouchStart(e) {
  if (e.touches.length === 2) {
    e.preventDefault();
    clearPressTimer();
    const rect = canvasRef.value.getBoundingClientRect();
    const center = touchCenterPos(e.touches, rect);
    pinchStartDistance = touchDistance(e.touches);
    pinchStartZoom = zoomIdx.value;
    pinchDate = hitToDate(center);
    selecting.value = false;
    selStartX.value = -1;
    selEndX.value = -1;
    return;
  }
  if (e.touches.length !== 1 || pinchStartDistance > 0) return;
  const rect = canvasRef.value.getBoundingClientRect();
  const x = touchAxisPos(e.touches[0], rect);
  const edge = edgeThreshold();
  const sel = getSelectionPixels();
  if (sel && (Math.abs(x - sel.x1) <= edge || Math.abs(x - sel.x2) <= edge)) {
    selStartX.value = Math.abs(x - sel.x1) <= edge ? sel.x2 : sel.x1;
    selEndX.value = x;
    selecting.value = true;
    e.preventDefault();
    return;
  }
  if (!props.vertical) {
    selStartX.value = x;
    selEndX.value = x;
    selecting.value = true;
    return;
  }
  pressPos = x;
  pressClientX = e.touches[0].clientX;
  pressClientY = e.touches[0].clientY;
  clearPressTimer();
  pressTimer = window.setTimeout(() => {
    pressTimer = 0;
    selStartX.value = pressPos;
    selEndX.value = pressPos;
    selecting.value = true;
    requestDraw();
  }, 380);
}

function onTouchMove(e) {
  if (e.touches.length === 2 && pinchStartDistance > 0) {
    e.preventDefault();
    const scale = touchDistance(e.touches) / pinchStartDistance;
    const indexDelta = Math.round(Math.log2(scale) * 4);
    const cx = (e.touches[0].clientX + e.touches[1].clientX) / 2;
    const cy = (e.touches[0].clientY + e.touches[1].clientY) / 2;
    setZoomIndex(pinchStartZoom + indexDelta, pinchDate, viewportAxisPos(cx, cy));
    return;
  }
  if (e.touches.length !== 1) return;
  const rect = canvasRef.value.getBoundingClientRect();
  const x = touchAxisPos(e.touches[0], rect);
  if (selecting.value) {
    selEndX.value = x;
    e.preventDefault();
    requestDraw();
    return;
  }
  if (pressTimer) {
    const dx = e.touches[0].clientX - pressClientX;
    const dy = e.touches[0].clientY - pressClientY;
    if (Math.hypot(dx, dy) > 12) clearPressTimer();
  }
}

function onTouchEnd(e) {
  if (pinchStartDistance > 0) {
    if (e.touches.length < 2) {
      pinchStartDistance = 0;
      pinchDate = null;
    }
    return;
  }
  if (pressTimer) {
    clearPressTimer();
    selStartX.value = pressPos;
    selEndX.value = pressPos;
    emitTapOrRange();
    selStartX.value = -1;
    selEndX.value = -1;
    requestDraw();
    return;
  }
  if (!selecting.value) return;
  selecting.value = false;
  emitTapOrRange();
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
  if (!wrapperRef.value) return;
  viewWidth.value = wrapperRef.value.clientWidth;
  viewHeight.value = props.vertical
    ? (wrapperRef.value.clientHeight || VERTICAL_H)
    : 96;
  requestDraw();
}

watch(() => props.buckets, () => {
  zoomIdx.value = props.vertical ? 3 : 2;
  nextTick(() => { scrollToEnd(); requestDraw(); });
});

watch([() => props.from, () => props.to, () => props.isDark], () => requestDraw());

onMounted(() => {
  measure();
  resizeObs = new ResizeObserver(measure);
  if (wrapperRef.value) resizeObs.observe(wrapperRef.value);
  nextTick(scrollToEnd);
});

onBeforeUnmount(() => {
  clearPressTimer();
  if (resizeObs) resizeObs.disconnect();
  if (drawRaf) cancelAnimationFrame(drawRaf);
});
</script>

<template>
  <div
    ref="wrapperRef"
    class="app-scrollbar w-full"
    :class="vertical
      ? 'timeline-chart-vertical overflow-x-hidden overflow-y-auto'
      : 'overflow-x-auto'"
    @scroll.passive="onScroll"
  >
    <div
      :style="vertical
        ? { minHeight: viewHeight + 'px', position: 'relative' }
        : { width: Math.max(stripWidth, viewWidth) + 'px', position: 'relative' }"
    >
      <canvas
        ref="canvasRef"
        class="block select-none"
        :class="vertical ? '' : 'sticky left-0'"
        :style="vertical
          ? { width: '100%', height: Math.max(stripWidth, viewHeight) + 'px', touchAction: 'pan-y' }
          : { width: viewWidth + 'px', height: '96px', touchAction: 'pan-y' }"
        @mousedown="onMouseDown"
        @mousemove="onMouseMove"
        @mouseup="onMouseUp"
        @mouseleave="onMouseLeave"
        @wheel="onWheel"
        @contextmenu="onContextMenu"
        @touchstart="onTouchStart"
        @touchmove="onTouchMove"
        @touchend="onTouchEnd"
        @touchcancel="onTouchEnd"
      />
    </div>
  </div>
</template>

<style scoped>
.timeline-chart-vertical {
  height: min(52dvh, 22.5rem);
  max-height: min(52dvh, 22.5rem);
  border-radius: 0.65rem;
  overscroll-behavior: contain;
  touch-action: pan-y;
}
</style>
