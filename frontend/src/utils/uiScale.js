export const UI_SCALE_MIN = 75;
export const UI_SCALE_MAX = 150;
export const UI_SCALE_DEFAULT = 100;
export const UI_SCALE_STEP = 5;

const STORAGE_KEY = "album-ui-scale";

let inflationFix = 1;
let currentPercent = UI_SCALE_DEFAULT;
let notifyTimer = 0;

export function normalizeUiScale(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return UI_SCALE_DEFAULT;
  const percent = n > 0 && n <= 2 ? Math.round(n * 100) : Math.round(n);
  const snapped = Math.round(percent / UI_SCALE_STEP) * UI_SCALE_STEP;
  return Math.min(UI_SCALE_MAX, Math.max(UI_SCALE_MIN, snapped));
}

export function loadUiScale() {
  try {
    return normalizeUiScale(localStorage.getItem(STORAGE_KEY) ?? UI_SCALE_DEFAULT);
  } catch {
    return UI_SCALE_DEFAULT;
  }
}

export function uiScalePercent() {
  return currentPercent;
}

export function effectiveUiZoom() {
  return (currentPercent / 100) * inflationFix;
}

function keyboardOpen(visualH, visualTop, innerH) {
  if (typeof document === "undefined") return false;
  const el = document.activeElement;
  const typing = Boolean(el && /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
  if (!typing) return false;
  const overlap = Math.max(0, innerH - visualH - visualTop);
  return overlap > 80;
}

export function layoutViewportSize() {
  const vv = typeof window === "undefined" ? null : window.visualViewport;
  const innerH = window.innerHeight || 0;
  const visualH = vv?.height || innerH;
  const visualTop = vv?.offsetTop || 0;
  const kb = keyboardOpen(visualH, visualTop, innerH);
  const height = kb ? visualH : (innerH || visualH);
  if (typeof document !== "undefined") {
    document.documentElement.classList.toggle("is-keyboard", kb);
  }
  return {
    height: Math.max(1, Math.round(height)),
    top: kb ? Math.max(0, Math.round(visualTop)) : 0,
  };
}

export function writeLayoutViewport() {
  if (typeof document === "undefined") return layoutViewportSize();
  const size = layoutViewportSize();
  const root = document.documentElement;
  root.style.setProperty("--app-height", `${size.height}px`);
  root.style.setProperty("--app-top", `${size.top}px`);
  return size;
}

function paintUiScale() {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  const user = currentPercent / 100;
  const effective = user * inflationFix;
  root.style.setProperty("--ui-scale", String(effective));
  root.style.setProperty("--ui-zoom-inv", "1");
  root.style.removeProperty("zoom");
  if (Math.abs(effective - 1) < 0.004) {
    root.classList.remove("is-ui-scaled");
  } else {
    root.classList.add("is-ui-scaled");
  }
  writeLayoutViewport();
  if (typeof window === "undefined") return;
  window.clearTimeout(notifyTimer);
  notifyTimer = window.setTimeout(() => {
    window.dispatchEvent(new Event("resize"));
  }, 16);
}

export function setUiScaleInflation(fix) {
  const next = Number(fix);
  inflationFix = Number.isFinite(next) && next > 0 ? next : 1;
  paintUiScale();
}

export function applyUiScale(value, { persist = true } = {}) {
  currentPercent = normalizeUiScale(value);
  if (persist) {
    try {
      localStorage.setItem(STORAGE_KEY, String(currentPercent));
    } catch {
      /* ignore quota / private mode */
    }
  }
  paintUiScale();
  return currentPercent;
}

export function initUiScale() {
  currentPercent = loadUiScale();
  paintUiScale();
  if (typeof window !== "undefined") {
    window.addEventListener("lumeno:booted", () => paintUiScale());
  }
  return currentPercent;
}
