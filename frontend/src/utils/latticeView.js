export const PINCH_STEP = 1.22;
export const PINCH_SNAP_DEADZONE = 0.07;
export const PINCH_MIN_SCALE = 0.28;
export const PINCH_MAX_SCALE = 3.6;

export function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

export function lerp(from, to, amount) {
  return from + (to - from) * amount;
}

export function levelFromScale(startLevel, scale, maxLevel) {
  const safeScale = Math.max(Number(scale) || 1, 0.01);
  return clamp(
    startLevel + Math.log(safeScale) / Math.log(PINCH_STEP),
    0,
    maxLevel,
  );
}

export function snapLevel(startLevel, scale, maxLevel) {
  if (!Number.isFinite(scale) || Math.abs(scale - 1) < PINCH_SNAP_DEADZONE) {
    return startLevel;
  }
  return clamp(Math.round(levelFromScale(startLevel, scale, maxLevel)), 0, maxLevel);
}

export function snapLevelByCellWidth({
  scale,
  startLevel,
  startCellW,
  layouts,
  viewportWidth,
  maxLevel,
}) {
  if (!Number.isFinite(scale) || Math.abs(scale - 1) < PINCH_SNAP_DEADZONE) {
    return startLevel;
  }
  const targetW = Math.max(1, startCellW) * Math.max(0.05, Number(scale) || 1);
  let best = startLevel;
  let bestScore = Infinity;
  const last = Math.min(maxLevel, Math.max(0, layouts.length - 1));
  for (let level = 0; level <= last; level += 1) {
    const layout = layouts[level] || layouts[0] || { cols: 1, gap: 0 };
    const width = cellWidth(viewportWidth, layout.cols, layout.gap);
    const score = Math.abs(Math.log(Math.max(1, width) / targetW));
    if (score < bestScore) {
      bestScore = score;
      best = level;
    }
  }
  return best;
}

/** Continuous pinch brackets: lerp layouts while cell size follows the fingers. */
export function pinchZoomBrackets(targetW, layouts, viewportWidth, maxLevel) {
  const last = Math.min(maxLevel, Math.max(0, (layouts?.length || 1) - 1));
  const widths = [];
  for (let level = 0; level <= last; level += 1) {
    const layout = layouts[level] || layouts[0] || { cols: 1, gap: 0 };
    widths.push(cellWidth(viewportWidth, layout.cols, layout.gap));
  }
  const minW = Math.max(1, widths[0] || 1);
  const maxW = Math.max(minW, widths[last] || minW);
  const rubber = 0.1;
  const cell = clamp(Math.max(1, Number(targetW) || 1), minW * (1 - rubber), maxW * (1 + rubber));
  if (cell <= widths[0]) return { lower: 0, upper: 0, amount: 0, cell };
  if (cell >= widths[last]) return { lower: last, upper: last, amount: 0, cell };

  let i = 0;
  while (i < last) {
    let j = i + 1;
    while (j < last && Math.abs(widths[j] - widths[i]) < 0.5) j += 1;
    const fromW = widths[i];
    const toW = widths[j];
    const lo = Math.min(fromW, toW);
    const hi = Math.max(fromW, toW);
    if (cell >= lo && cell <= hi) {
      const amount = Math.abs(toW - fromW) < 0.5 ? 0 : (cell - fromW) / (toW - fromW);
      return { lower: i, upper: j, amount: clamp(amount, 0, 1), cell };
    }
    i = j;
  }
  return { lower: last, upper: last, amount: 0, cell };
}

export function pinchScale(scale) {
  return clamp(Number(scale) || 1, PINCH_MIN_SCALE, PINCH_MAX_SCALE);
}

export function boardMotion({ startFocalX, startFocalY, focalX, focalY, scale }) {
  const liveScale = pinchScale(scale);
  return {
    originX: startFocalX,
    originY: startFocalY,
    translateX: (Number(focalX) || 0) - startFocalX,
    translateY: (Number(focalY) || 0) - startFocalY,
    scale: liveScale,
  };
}

export function boardTransformCss(motion) {
  return `translate3d(${motion.translateX}px, ${motion.translateY}px, 0) scale(${motion.scale})`;
}

/** Blend top/bottom edge pinning during wheel zoom so head/tail do not clip-jump. */
export function edgeTranslateY(scale, originY, scrollOffset, scrollSize, viewportH, edgeZone = 120) {
  const s = Number(scale) || 1;
  const O = Number(originY) || 0;
  const S = Math.max(0, Number(scrollOffset) || 0);
  const H = Math.max(0, Number(scrollSize) || 0);
  const V = Math.max(1, Number(viewportH) || 1);
  const maxScroll = Math.max(0, H - V);
  if (maxScroll <= 0) return -O * (1 - s);
  const zone = Math.min(Math.max(1, edgeZone), maxScroll);
  const headBlend = clamp(1 - S / zone, 0, 1);
  const tailBlend = clamp((S - (maxScroll - zone)) / zone, 0, 1);
  const pinTop = -O * (1 - s);
  const pinBottom = (V - O) * (1 - s);
  return headBlend * pinTop + tailBlend * pinBottom;
}

export function gridZoomTransformCss(scale, translateY) {
  return `translate3d(0, ${translateY}px, 0) scale(${scale})`;
}

export function overscanRows(viewportHeight, rowPitch, minScale = PINCH_MIN_SCALE) {
  const pitch = Math.max(1, Number(rowPitch) || 1);
  const cover = (Number(viewportHeight) || 0) / Math.max(0.2, minScale);
  return Math.max(4, Math.ceil(cover / pitch) + 2);
}

export function cellWidth(viewportWidth, cols, gap) {
  const count = Math.max(1, Number(cols) || 1);
  const safeGap = Math.max(0, Number(gap) || 0);
  return Math.max(1, (viewportWidth - safeGap * (count - 1)) / count);
}

/**
 * Partial first-row length that places `lead` (the item at dest col 0 of the
 * focused row) after `floor(lead / cols)` full rows. 0 means the album starts
 * flush at column 0.
 */
export function headRemainderForLead(lead, cols) {
  const n = Math.max(1, Number(cols) || 1);
  const start = Math.max(0, Number(lead) || 0);
  if (start <= 0) return 0;
  return start % n;
}

export function itemCell(index, cols, headRemainder = 0) {
  const n = Math.max(1, Number(cols) || 1);
  const i = Math.max(0, Number(index) || 0);
  const head = Math.max(0, Number(headRemainder) || 0);
  if (head > 0) {
    if (i < head) return { row: 0, col: i };
    const after = i - head;
    return { row: 1 + Math.floor(after / n), col: after % n };
  }
  return { row: Math.floor(i / n), col: i % n };
}

export function cellIndex(row, col, cols, headRemainder, total) {
  const n = Math.max(1, Number(cols) || 1);
  const r = Number(row);
  const c = Number(col);
  if (!Number.isFinite(r) || !Number.isFinite(c) || r < 0 || c < 0 || c >= n) return -1;
  const head = Math.max(0, Number(headRemainder) || 0);
  let index;
  if (head > 0) {
    if (r === 0) {
      if (c >= head) return -1;
      index = c;
    } else {
      index = head + (r - 1) * n + c;
    }
  } else {
    index = r * n + c;
  }
  const last = Math.max(0, Number(total) || 0);
  if (index < 0 || index >= last) return -1;
  return index;
}

export function rowItems(row, cols, headRemainder, total) {
  const n = Math.max(1, Number(cols) || 1);
  const items = [];
  for (let col = 0; col < n; col += 1) {
    const index = cellIndex(row, col, n, headRemainder, total);
    if (index >= 0) items.push(index);
  }
  return items;
}

export function rowCountFor(total, cols, headRemainder = 0) {
  const n = Math.max(1, Number(cols) || 1);
  const count = Math.max(0, Number(total) || 0);
  if (count === 0) return 0;
  const head = Math.max(0, Number(headRemainder) || 0);
  if (head > 0) {
    const first = Math.min(head, count);
    return 1 + Math.ceil((count - first) / n);
  }
  return Math.ceil(count / n);
}

export function buildGridRows(items, cols, headRemainder = 0) {
  const n = Math.max(1, Number(cols) || 1);
  const list = Array.isArray(items) ? items : [];
  const result = [];
  if (!list.length) return result;
  let offset = 0;
  const head = Math.max(0, Number(headRemainder) || 0);
  if (head > 0) {
    const first = Math.min(head, list.length);
    result.push({ startIndex: 0, items: list.slice(0, first) });
    offset = first;
  }
  for (; offset < list.length; offset += n) {
    result.push({ startIndex: offset, items: list.slice(offset, offset + n) });
  }
  return result;
}

/** Map a column on the focused from-row onto the dest row, keeping the focal photo's relative X. */
export function mapFocalCol(fromCol, fromCount, destCols) {
  const fromN = Math.max(1, Number(fromCount) || 1);
  const destN = Math.max(1, Number(destCols) || 1);
  const col = clamp(Number(fromCol) || 0, 0, fromN - 1);
  if (fromN <= 1) return 0;
  if (destN <= 1) return 0;
  return Math.round(col * (destN - 1) / (fromN - 1));
}

export function scaleOriginX({
  fromPitch,
  destPitch,
  fromOriginX = 0,
  destOriginX = 0,
  keepFromCol = 0,
  keepStartCol = 0,
}) {
  const fromP = Math.max(1, Number(fromPitch) || 1);
  const destP = Math.max(1, Number(destPitch) || 1);
  const s = destP / fromP;
  const fromX0 = (Number(fromOriginX) || 0) + keepFromCol * fromP;
  const destX0 = (Number(destOriginX) || 0) + keepStartCol * destP;
  if (Math.abs(1 - s) < 1e-6) return fromX0;
  return (destX0 - fromX0 * s) / (1 - s);
}

function alignedPlan(total, destCols, focalIndex, edge) {
  const n = Math.max(1, Number(destCols) || 1);
  const last = Math.max(0, (Number(total) || 0) - 1);
  const idx = clamp(Number(focalIndex) || 0, 0, last);
  const row = Math.floor(idx / n);
  const keepItems = rowItems(row, n, 0, total);
  return {
    edge,
    destCols: n,
    headRemainder: 0,
    lead: row * n,
    keepItems,
    keepStartCol: 0,
    keepFromCol0: 0,
    destAnchorRow: row,
    destFocalCol: itemCell(idx, n, 0).col,
  };
}

/**
 * Apple Photos-style remap: keep the focused row's photos as an anchor block,
 * then count full dest rows + remainder before/after that block. Falls back to
 * a flush-from-zero layout when the dest row would hit the album head/tail.
 */
export function planRowAnchor({
  total,
  fromCols,
  fromHeadRemainder = 0,
  destCols,
  focalIndex,
  viewportH = 0,
  destPitch = 1,
  destCellH = 0,
  anchorRowY = 0,
  edgeSlop = 8,
} = {}) {
  const destN = Math.max(1, Number(destCols) || 1);
  const fromN = Math.max(1, Number(fromCols) || 1);
  const count = Math.max(0, Number(total) || 0);
  if (count === 0) {
    return alignedPlan(0, destN, 0, "aligned");
  }
  const focal = clamp(Number(focalIndex) || 0, 0, count - 1);
  const fromCell = itemCell(focal, fromN, fromHeadRemainder);
  const fromRowIndices = rowItems(fromCell.row, fromN, fromHeadRemainder, count);
  if (!fromRowIndices.length) {
    return alignedPlan(count, destN, focal, "aligned");
  }

  const fromCount = fromRowIndices.length;
  const fromCol = Math.max(0, fromRowIndices.indexOf(focal));
  const destFocalCol = mapFocalCol(fromCol, fromCount, destN);

  let keepItems;
  let keepStartCol;
  let keepFromCol0;
  if (destN >= fromCount) {
    keepItems = fromRowIndices;
    keepFromCol0 = 0;
    keepStartCol = clamp(destFocalCol - fromCol, 0, destN - fromCount);
  } else {
    keepFromCol0 = clamp(fromCol - destFocalCol, 0, fromCount - destN);
    keepItems = fromRowIndices.slice(keepFromCol0, keepFromCol0 + destN);
    keepStartCol = 0;
  }

  const lead = keepItems[0] - keepStartCol;
  if (lead < 0) {
    return alignedPlan(count, destN, focal, "head");
  }

  const headRemainder = headRemainderForLead(lead, destN);
  const destCell = itemCell(keepItems[0], destN, headRemainder);
  const rows = rowCountFor(count, destN, headRemainder);
  const pitch = Math.max(1, Number(destPitch) || 1);
  const cellH = Math.max(1, Number(destCellH) || pitch);
  const viewH = Math.max(0, Number(viewportH) || 0);
  const rowY = Number(anchorRowY) || 0;
  const slop = Math.max(0, Number(edgeSlop) || 0);
  const above = destCell.row * pitch;
  if (viewH > 0 && above + slop < rowY) {
    return alignedPlan(count, destN, focal, "head");
  }
  const below = Math.max(0, rows - destCell.row - 1) * pitch;
  const needBelow = Math.max(0, viewH - rowY - cellH);
  if (viewH > 0 && below + slop < needBelow) {
    return alignedPlan(count, destN, focal, "tail");
  }

  return {
    edge: "none",
    destCols: destN,
    headRemainder,
    lead,
    keepItems,
    keepStartCol,
    keepFromCol0,
    destAnchorRow: destCell.row,
    destFocalCol: destCell.col + (focal - keepItems[0]),
  };
}

export function destIndexAt(plan, destRow, destCol, total) {
  if (!plan) return -1;
  return cellIndex(destRow, destCol, plan.destCols, plan.headRemainder, total);
}

export function fromColToDestCol(plan, fromCol) {
  if (!plan) return -1;
  const offset = fromCol - (plan.keepFromCol0 || 0);
  if (offset < 0 || offset >= (plan.keepItems?.length || 0)) return -1;
  return plan.keepStartCol + offset;
}

export function metricsForLevel(level, levelLayouts, viewportWidth, cellAspect = 1) {
  const maxLevel = Math.max(0, levelLayouts.length - 1);
  const bounded = clamp(level, 0, maxLevel);
  const lowerIndex = Math.floor(bounded);
  const upperIndex = Math.ceil(bounded);
  const amount = bounded - lowerIndex;
  const lower = levelLayouts[lowerIndex] || levelLayouts[0] || { cols: 1, gap: 0 };
  const upper = levelLayouts[upperIndex] || lower;
  const lowerWidth = cellWidth(viewportWidth, lower.cols, lower.gap);
  const upperWidth = cellWidth(viewportWidth, upper.cols, upper.gap);
  const width = lerp(lowerWidth, upperWidth, amount);
  const gap = lerp(lower.gap || 0, upper.gap || 0, amount);
  const height = width * Math.max(0.01, Number(cellAspect) || 1);

  return {
    level: bounded,
    lowerIndex,
    upperIndex,
    amount,
    cols: lerp(lower.cols || 1, upper.cols || 1, amount),
    gap,
    cellW: width,
    cellH: height,
    colPitch: width + gap,
    rowPitch: height + gap,
  };
}
