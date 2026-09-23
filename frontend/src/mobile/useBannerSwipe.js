import { computed, onBeforeUnmount, ref } from "vue";

const LOCK_PX = 8;
const DISMISS_PX = 28;
const DISMISS_VELOCITY = 0.42;
const EXIT_Y = -132;
const EXIT_MS = 180;

export function useBannerSwipe({ onDismiss, onDragStart, onSettle } = {}) {
  const y = ref(0);
  const dragging = ref(false);
  const exiting = ref(false);
  const settling = ref(false);
  let tracking = false;
  let startX = 0;
  let startY = 0;
  let startAt = 0;
  let lastY = 0;
  let lastAt = 0;
  let pointerId = null;
  let moved = false;
  let ignoreClick = false;
  let exitTimer = 0;

  const style = computed(() => {
    if (!dragging.value && !exiting.value && !settling.value && y.value === 0) return {};
    return {
      transform: `translate3d(-50%, ${y.value}px, 0)`,
      opacity: String(y.value < 0 ? Math.max(0, 1 + y.value / 90) : 1),
    };
  });

  function unbind() {
    window.removeEventListener("pointermove", onPointerMove);
    window.removeEventListener("pointerup", onPointerUp);
    window.removeEventListener("pointercancel", onPointerUp);
  }

  function reset() {
    tracking = false;
    dragging.value = false;
    pointerId = null;
    moved = false;
    unbind();
  }

  function snapBack() {
    exiting.value = false;
    dragging.value = false;
    if (Math.abs(y.value) < 1) {
      y.value = 0;
      settling.value = false;
      onSettle?.();
      return;
    }
    settling.value = true;
    requestAnimationFrame(() => {
      y.value = 0;
    });
    window.setTimeout(() => {
      settling.value = false;
      onSettle?.();
    }, EXIT_MS);
  }

  function finishDismiss() {
    window.clearTimeout(exitTimer);
    exiting.value = true;
    dragging.value = false;
    y.value = EXIT_Y;
    exitTimer = window.setTimeout(() => {
      exitTimer = 0;
      onDismiss?.();
      y.value = 0;
      exiting.value = false;
    }, EXIT_MS);
  }

  function onPointerDown(event) {
    if (exiting.value) return;
    if (event.pointerType === "mouse" && event.button !== 0) return;
    tracking = true;
    dragging.value = false;
    moved = false;
    ignoreClick = false;
    pointerId = event.pointerId;
    startX = event.clientX;
    startY = event.clientY;
    lastY = startY;
    startAt = performance.now();
    lastAt = startAt;
    window.addEventListener("pointermove", onPointerMove, { passive: false });
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
  }

  function onPointerMove(event) {
    if (!tracking || event.pointerId !== pointerId) return;
    const dy = event.clientY - startY;
    const dx = event.clientX - startX;
    if (!dragging.value) {
      if (Math.abs(dy) < LOCK_PX && Math.abs(dx) < LOCK_PX) return;
      if (Math.abs(dx) > Math.abs(dy) * 1.2) {
        reset();
        return;
      }
      dragging.value = true;
      moved = true;
      ignoreClick = true;
      onDragStart?.();
      try {
        event.target?.setPointerCapture?.(event.pointerId);
      } catch {
        // Pointer may already have ended.
      }
    }
    lastY = event.clientY;
    lastAt = performance.now();
    y.value = dy < 0 ? dy : dy * 0.18;
    if (event.cancelable) event.preventDefault();
  }

  function onPointerUp(event) {
    if (!tracking || (event.pointerId != null && event.pointerId !== pointerId)) return;
    const flicked = dragging.value;
    const dy = y.value;
    const vx = (lastY - startY) / Math.max(1, lastAt - startAt);
    reset();
    if (!flicked) {
      snapBack();
      return;
    }
    if (dy < -DISMISS_PX || vx < -DISMISS_VELOCITY) {
      finishDismiss();
      return;
    }
    snapBack();
  }

  function onClick(event) {
    if (!ignoreClick) return false;
    event.preventDefault();
    event.stopPropagation();
    ignoreClick = false;
    return true;
  }

  function clear() {
    window.clearTimeout(exitTimer);
    exitTimer = 0;
    exiting.value = false;
    settling.value = false;
    y.value = 0;
    reset();
  }

  onBeforeUnmount(clear);

  return {
    y,
    dragging,
    exiting,
    style,
    onPointerDown,
    onClick,
    clear,
  };
}
