import { ref } from "vue";

/**
 * Horizontal swipe-to-reveal that stays out of the way of native scrolling.
 * Listeners are passive: vertical pans scroll, only a clear horizontal move opens actions.
 */
export function useSwipeReveal(actionWidth = 148) {
  const revealId = ref(null);
  const revealX = ref(0);
  const draggingId = ref(null);
  const locked = ref(false);
  let gen = 0;

  function closeReveal() {
    revealId.value = null;
    revealX.value = 0;
  }

  function suppressGhostClick() {
    const stop = (event) => {
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener("click", stop, true);
    window.setTimeout(() => window.removeEventListener("click", stop, true), 480);
  }

  function lock() {
    locked.value = true;
    gen += 1;
    draggingId.value = null;
    revealX.value = 0;
    closeReveal();
  }

  function unlock() {
    window.setTimeout(() => {
      locked.value = false;
    }, 40);
  }

  function rowOffset(id) {
    if (draggingId.value === id) return revealX.value;
    if (revealId.value === id) return -actionWidth;
    return 0;
  }

  function onSwipeDown(event, id, { ignore, onTap } = {}) {
    if (locked.value) return;
    if (event.pointerType === "mouse" && event.button !== 0) return;
    if (ignore && event.target.closest?.(ignore)) return;

    const startX = event.clientX;
    const startY = event.clientY;
    const myGen = ++gen;
    let tracking = true;
    let horizontal = false;
    const origin = revealId.value === id ? -actionWidth : 0;

    function cleanup() {
      tracking = false;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    }

    function move(ev) {
      if (!tracking || myGen !== gen || ev.pointerId !== event.pointerId) return;
      if (locked.value) {
        cleanup();
        draggingId.value = null;
        revealX.value = 0;
        return;
      }
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      if (!horizontal) {
        if (Math.abs(dx) < 14 && Math.abs(dy) < 14) return;
        if (Math.abs(dy) >= Math.abs(dx)) {
          cleanup();
          draggingId.value = null;
          revealX.value = 0;
          return;
        }
        horizontal = true;
        draggingId.value = id;
        if (revealId.value && revealId.value !== id) revealId.value = null;
      }
      revealX.value = Math.min(0, Math.max(-actionWidth - 24, origin + dx));
    }

    function up(ev) {
      if (!tracking) return;
      if (ev.pointerId != null && ev.pointerId !== event.pointerId) return;
      if (myGen !== gen) {
        cleanup();
        return;
      }
      cleanup();
      draggingId.value = null;
      if (locked.value) {
        revealX.value = 0;
        return;
      }
      if (!horizontal) {
        revealX.value = 0;
        const tapped = Math.abs(ev.clientX - startX) < 8 && Math.abs(ev.clientY - startY) < 8;
        if (tapped) {
          if (origin < 0) closeReveal();
          else {
            suppressGhostClick();
            onTap?.();
          }
        }
        return;
      }
      revealId.value = revealX.value < -actionWidth * 0.45 ? id : null;
      revealX.value = 0;
    }

    window.addEventListener("pointermove", move, { passive: true });
    window.addEventListener("pointerup", up, { passive: true });
    window.addEventListener("pointercancel", up, { passive: true });
  }

  return {
    revealId,
    revealX,
    draggingId,
    closeReveal,
    lock,
    unlock,
    rowOffset,
    onSwipeDown,
  };
}
