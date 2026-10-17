import { onBeforeUnmount, ref, watch } from "vue";

const MOVE_SLOP = 16;

/**
 * Filter rows are long-pressed to reorder. touch-action: none keeps that
 * gesture from panning the sheet; a quick vertical swipe still scrolls it.
 */
export function useReorderScroll() {
  const scrollEl = ref(null);
  const reordering = ref(false);
  let tracking = false;
  let axis = "";
  let startX = 0;
  let startY = 0;
  let lastY = 0;

  function blockScroll(event) {
    if (event.cancelable) event.preventDefault();
  }

  function onTouchStart(event) {
    if (event.touches.length !== 1) return;
    const touch = event.touches[0];
    tracking = true;
    axis = "";
    startX = touch.clientX;
    startY = lastY = touch.clientY;
  }

  function onTouchMove(event) {
    if (!tracking || event.touches.length !== 1) return;
    if (reordering.value) {
      if (event.cancelable) event.preventDefault();
      return;
    }
    const touch = event.touches[0];
    const dx = touch.clientX - startX;
    const dy = touch.clientY - startY;
    if (!axis) {
      if (Math.abs(dx) < MOVE_SLOP && Math.abs(dy) < MOVE_SLOP) return;
      axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y";
    }
    if (axis !== "y") return;
    const el = scrollEl.value;
    if (!el) return;
    el.scrollTop += lastY - touch.clientY;
    lastY = touch.clientY;
    if (event.cancelable) event.preventDefault();
  }

  function onTouchEnd() {
    tracking = false;
    axis = "";
  }

  function onChoose() {
    reordering.value = true;
    tracking = false;
    document.addEventListener("touchmove", blockScroll, { capture: true, passive: false });
  }

  function onUnchoose() {
    reordering.value = false;
    document.removeEventListener("touchmove", blockScroll, { capture: true });
  }

  watch(scrollEl, (el, prev) => {
    prev?.removeEventListener("touchmove", onTouchMove);
    el?.addEventListener("touchmove", onTouchMove, { passive: false });
  });

  onBeforeUnmount(() => {
    scrollEl.value?.removeEventListener("touchmove", onTouchMove);
    document.removeEventListener("touchmove", blockScroll, { capture: true });
  });

  return {
    scrollEl,
    reordering,
    onTouchStart,
    onTouchEnd,
    onChoose,
    onUnchoose,
  };
}
