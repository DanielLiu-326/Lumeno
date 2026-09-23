import { computed, onBeforeUnmount, ref, watch } from "vue";

const DISMISS_PX = 96;
const DISMISS_VELOCITY = 0.55;
const EXPAND_PX = 64;
const EXPAND_VELOCITY = 0.45;
const COLLAPSE_PX = 72;
const COLLAPSE_VELOCITY = 0.5;

function viewportH() {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--app-height");
  const fromCss = parseFloat(raw);
  if (Number.isFinite(fromCss) && fromCss > 0) return Math.round(fromCss);
  return Math.max(1, Math.round(window.visualViewport?.height || window.innerHeight));
}

function canScrollY(el) {
  if (!(el instanceof Element)) return false;
  const mode = getComputedStyle(el).overflowY;
  if (mode !== "auto" && mode !== "scroll" && mode !== "overlay") return false;
  return el.scrollHeight > el.clientHeight + 1;
}

function inNestedScrollY(target, root) {
  let el = target instanceof Element ? target : target?.parentElement;
  while (el && el !== root) {
    if (canScrollY(el)) return true;
    el = el.parentElement;
  }
  return false;
}

export function useSheetDrag(openRef, closeFn) {
  const dragY = ref(0);
  const heightPx = ref(null);
  const dragging = ref(false);
  const expanded = ref(false);
  let tracking = false;
  let startX = 0;
  let startY = 0;
  let startAt = 0;
  let lastY = 0;
  let lastAt = 0;
  let pointerId = null;
  let panelEl = null;
  let compactH = 0;
  let fullH = 0;
  let extraH = 0;
  let settleTimer = 0;
  let moved = false;

  const panelStyle = computed(() => {
    const style = {
      "--sheet-drag": `${dragY.value}px`,
    };
    if (heightPx.value != null) {
      style.height = `${heightPx.value}px`;
      style.maxHeight = `${Math.max(fullH, heightPx.value)}px`;
    }
    return style;
  });
  const maskStyle = computed(() => ({
    opacity: String(Math.max(0.06, 1 - dragY.value / 420)),
  }));

  function unbind() {
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
    window.removeEventListener("pointercancel", onPointerUp);
  }

  function clearSettle() {
    window.clearTimeout(settleTimer);
    settleTimer = 0;
  }

  function reset() {
    tracking = false;
    dragging.value = false;
    pointerId = null;
    panelEl = null;
    dragY.value = 0;
    extraH = 0;
    moved = false;
    unbind();
  }

  function fullReset() {
    reset();
    expanded.value = false;
    heightPx.value = null;
    compactH = 0;
    fullH = 0;
    clearSettle();
  }

  function measure(panel) {
    const root = panel.closest(".mobile-sheet-root");
    fullH = Math.round(root?.clientHeight || viewportH());
    if (!expanded.value) compactH = panel.offsetHeight;
    compactH = Math.min(Math.max(1, compactH), fullH);
  }

  function applyDrag(dy) {
    const range = Math.max(0, fullH - compactH);
    if (expanded.value) {
      if (dy <= 0) {
        extraH = range;
        dragY.value = 0;
        heightPx.value = fullH;
        return;
      }
      if (dy < range) {
        extraH = range - dy;
        dragY.value = 0;
        heightPx.value = compactH + extraH;
        return;
      }
      extraH = 0;
      dragY.value = dy - range;
      heightPx.value = compactH;
      return;
    }
    if (dy >= 0) {
      extraH = 0;
      dragY.value = dy;
      heightPx.value = null;
      return;
    }
    extraH = Math.min(range, -dy);
    dragY.value = 0;
    heightPx.value = extraH > 0 ? compactH + extraH : null;
  }

  function snapExpanded() {
    expanded.value = true;
    extraH = Math.max(0, fullH - compactH);
    dragY.value = 0;
    heightPx.value = fullH;
    clearSettle();
    settleTimer = window.setTimeout(() => {
      if (expanded.value && !dragging.value) heightPx.value = null;
    }, 260);
  }

  function snapCompact() {
    expanded.value = false;
    extraH = 0;
    dragY.value = 0;
    heightPx.value = compactH;
    clearSettle();
    settleTimer = window.setTimeout(() => {
      if (!expanded.value && !dragging.value) heightPx.value = null;
    }, 260);
  }

  function suppressClick() {
    const stop = (event) => {
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener("click", stop, true);
    window.setTimeout(() => window.removeEventListener("click", stop, true), 0);
  }

  function onPointerDown(event) {
    if (event.pointerType === "mouse" && event.button !== 0) return;
    if (event.target.closest?.(".mobile-sheet-view, .app-slider, .mobile-filter-swipe, .mobile-filter-card, .mobile-sort-row, .mobile-filter-add, .mobile-filter-editor, .mobile-sheet-scroll, .mobile-filter-collections, input, textarea, select")) return;
    const panel = event.currentTarget;
    if (inNestedScrollY(event.target, panel)) return;
    tracking = true;
    dragging.value = false;
    moved = false;
    pointerId = event.pointerId;
    panelEl = panel;
    startX = event.clientX;
    startY = event.clientY;
    lastY = startY;
    startAt = performance.now();
    lastAt = startAt;
    dragY.value = 0;
    extraH = expanded.value ? Math.max(0, fullH - compactH) : 0;
    measure(panel);
    window.addEventListener("pointermove", onPointerMove, { passive: false });
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
  }

  function onPointerMove(event) {
    if (!tracking || event.pointerId !== pointerId) return;
    const dy = event.clientY - startY;
    const dx = event.clientX - startX;
    if (!dragging.value) {
      if (Math.abs(dy) < 8) {
        if (Math.abs(dx) > 12 && Math.abs(dx) > Math.abs(dy)) reset();
        return;
      }
      dragging.value = true;
      moved = true;
      try {
        panelEl?.setPointerCapture?.(event.pointerId);
      } catch {
        // Capture can fail if the pointer already ended.
      }
    }
    lastY = event.clientY;
    lastAt = performance.now();
    applyDrag(dy);
    if (event.cancelable) event.preventDefault();
  }

  function onPointerUp(event) {
    if (!tracking || (event.pointerId != null && event.pointerId !== pointerId)) return;
    const flicked = dragging.value;
    const y = dragY.value;
    const flick = (lastY - startY) / Math.max(1, lastAt - startAt);
    const range = Math.max(0, fullH - compactH);
    const grown = extraH;
    tracking = false;
    dragging.value = false;
    pointerId = null;
    panelEl = null;
    unbind();
    if (moved) suppressClick();
    if (!flicked) {
      if (expanded.value) heightPx.value = null;
      return;
    }
    if (expanded.value) {
      if (y > DISMISS_PX || flick > DISMISS_VELOCITY) {
        closeFn();
        return;
      }
      const currentH = heightPx.value ?? fullH;
      if (fullH - currentH > COLLAPSE_PX || flick > COLLAPSE_VELOCITY) {
        snapCompact();
        return;
      }
      snapExpanded();
      return;
    }
    if (y > DISMISS_PX || flick > DISMISS_VELOCITY) {
      closeFn();
      return;
    }
    if (grown > EXPAND_PX || (range > 0 && grown > range * 0.28) || flick < -EXPAND_VELOCITY) {
      snapExpanded();
      return;
    }
    snapCompact();
  }

  watch(openRef, (open) => {
    if (open) {
      expanded.value = false;
      heightPx.value = null;
      extraH = 0;
      dragY.value = 0;
      return;
    }
    fullReset();
  });

  onBeforeUnmount(fullReset);

  const raised = computed(() => expanded.value || heightPx.value != null);
  return { dragging, expanded: raised, panelStyle, maskStyle, onPointerDown };
}
