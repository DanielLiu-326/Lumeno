import { computed, onBeforeUnmount, ref } from "vue";

const MIN_SCALE = 1;
const MAX_SCALE = 5;
const SNAP = 1.04;
const DOUBLE_MS = 280;
const TAP_PX = 12;
const SLIDE_LOCK_PX = 10;

function distance(a, b) {
  return Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY);
}

function midpoint(a, b) {
  return {
    x: (a.clientX + b.clientX) / 2,
    y: (a.clientY + b.clientY) / 2,
  };
}

function clamp(n, min, max) {
  return Math.min(max, Math.max(min, n));
}

export function usePhotoPinch({
  onTap,
  onSwipeUp,
  onSwipeDown,
  onDismissMove,
  onSlideStart,
  onSlideEnd,
  onSlideGrab,
  onSlidePage,
  canSlidePrev,
  canSlideNext,
  canDismiss,
  isLocked,
  allowPinch,
} = {}) {
  const scale = ref(1);
  const x = ref(0);
  const y = ref(0);
  const slideX = ref(0);
  const active = ref(false);
  const sliding = ref(false);
  const zoomed = computed(() => scale.value > SNAP);

  const imgStyle = computed(() => ({
    transform: `translate3d(${x.value}px, ${y.value}px, 0) scale(${scale.value})`,
  }));

  let mode = "idle";
  let pinch = null;
  let pan = null;
  let tapOrigin = null;
  let lastTapAt = 0;
  let lastTapX = 0;
  let lastTapY = 0;
  let tapTimer = 0;
  let moved = false;
  let lastMoveX = 0;
  let lastMoveY = 0;
  let lastMoveT = 0;
  let velocityX = 0;
  let velocityY = 0;
  let slideOriginX = 0;
  let lift = null;
  let coastRaf = 0;

  function stageBox(el) {
    const rect = el?.getBoundingClientRect();
    if (!rect) return { w: 1, h: 1, left: 0, top: 0, cx: 0.5, cy: 0.5 };
    return {
      w: rect.width,
      h: rect.height,
      left: rect.left,
      top: rect.top,
      cx: rect.width / 2,
      cy: rect.height / 2,
    };
  }

  function clampPan(nx, ny, s, box) {
    if (s <= SNAP) return { x: 0, y: 0 };
    const maxX = (box.w * (s - 1)) / 2;
    const maxY = (box.h * (s - 1)) / 2;
    return {
      x: clamp(nx, -maxX, maxX),
      y: clamp(ny, -maxY, maxY),
    };
  }

  function zoomAround(nextScale, focalX, focalY, box, fromScale = scale.value, fromX = x.value, fromY = y.value) {
    const s = clamp(nextScale, MIN_SCALE, MAX_SCALE);
    const ratio = s / Math.max(fromScale, 0.01);
    const nx = focalX - box.cx - (focalX - box.cx - fromX) * ratio;
    const ny = focalY - box.cy - (focalY - box.cy - fromY) * ratio;
    const panTo = clampPan(nx, ny, s, box);
    scale.value = s;
    x.value = panTo.x;
    y.value = panTo.y;
  }

  function rubberEnds(x, width) {
    const prevOk = canSlidePrev?.() !== false;
    const nextOk = canSlideNext?.() !== false;
    if (x < 0 && !nextOk) return x * 0.32;
    if (x > 0 && !prevOk) return x * 0.32;
    if (x < -width) return -width + (x + width) * 0.32;
    if (x > width) return width + (x - width) * 0.32;
    return x;
  }

  function pullSlide(raw, width) {
    let x = raw;
    if (x <= -width && canSlideNext?.() && onSlidePage?.("next") !== false) {
      x += width;
      slideOriginX -= width;
    } else if (x >= width && canSlidePrev?.() && onSlidePage?.("prev") !== false) {
      x -= width;
      slideOriginX += width;
    }
    slideX.value = rubberEnds(x, width);
  }

  function stopCoast() {
    if (!coastRaf) return;
    cancelAnimationFrame(coastRaf);
    coastRaf = 0;
  }

  function startPanCoast(el, vx, vy) {
    stopCoast();
    const speed = Math.hypot(vx, vy);
    if (!el || speed < 0.05 || scale.value <= SNAP) return;
    const cap = 2.6;
    if (speed > cap) {
      const limit = cap / speed;
      vx *= limit;
      vy *= limit;
    }
    active.value = true;
    let last = performance.now();
    const step = (now) => {
      if (scale.value <= SNAP) {
        coastRaf = 0;
        active.value = false;
        return;
      }
      const dt = Math.min(34, Math.max(0, now - last));
      last = now;
      const decay = Math.exp(-0.0028 * dt);
      vx *= decay;
      vy *= decay;
      const box = stageBox(el);
      const rawX = x.value + vx * dt;
      const rawY = y.value + vy * dt;
      const next = clampPan(rawX, rawY, scale.value, box);
      if (next.x !== rawX) vx = 0;
      if (next.y !== rawY) vy = 0;
      x.value = next.x;
      y.value = next.y;
      if ((vx === 0 && vy === 0) || Math.hypot(vx, vy) < 0.012) {
        coastRaf = 0;
        active.value = false;
        return;
      }
      coastRaf = requestAnimationFrame(step);
    };
    coastRaf = requestAnimationFrame(step);
  }

  function resetSlide() {
    sliding.value = false;
    slideX.value = 0;
    velocityX = 0;
  }

  function resetZoom() {
    stopCoast();
    mode = "idle";
    pinch = null;
    pan = null;
    tapOrigin = null;
    moved = false;
    active.value = false;
    lift = null;
    scale.value = 1;
    x.value = 0;
    y.value = 0;
    window.clearTimeout(tapTimer);
    tapTimer = 0;
  }

  function reset() {
    resetZoom();
    resetSlide();
  }

  function beginPinch(event) {
    const a = event.touches[0];
    const b = event.touches[1];
    if (!a || !b) return;
    stopCoast();
    velocityX = 0;
    velocityY = 0;
    if (sliding.value) {
      sliding.value = false;
      onSlideEnd?.({ dx: slideX.value, vx: 0, width: stageBox(event.currentTarget).w, cancelled: true });
    }
    const box = stageBox(event.currentTarget);
    const mid = midpoint(a, b);
    pinch = {
      dist: Math.max(1, distance(a, b)),
      scale: scale.value,
      x: x.value,
      y: y.value,
      fx: mid.x - box.left,
      fy: mid.y - box.top,
    };
    mode = "pinch";
    active.value = true;
    moved = true;
    window.clearTimeout(tapTimer);
    tapTimer = 0;
  }

  function applyPinch(event) {
    if (!pinch || event.touches.length < 2) return;
    const box = stageBox(event.currentTarget);
    const next = Math.max(1, distance(event.touches[0], event.touches[1])) / pinch.dist;
    const s = clamp(pinch.scale * next, 0.86, MAX_SCALE + 0.35);
    const mid = midpoint(event.touches[0], event.touches[1]);
    const fx = mid.x - box.left;
    const fy = mid.y - box.top;
    const contentX = (pinch.fx - box.cx - pinch.x) / Math.max(pinch.scale, 0.01);
    const contentY = (pinch.fy - box.cy - pinch.y) / Math.max(pinch.scale, 0.01);
    const panTo = clampPan(fx - box.cx - contentX * s, fy - box.cy - contentY * s, Math.max(s, MIN_SCALE), box);
    scale.value = s;
    x.value = s < MIN_SCALE ? fx - box.cx - contentX * s : panTo.x;
    y.value = s < MIN_SCALE ? fy - box.cy - contentY * s : panTo.y;
  }

  function endPinch(event) {
    const box = stageBox(event.currentTarget);
    if (scale.value < SNAP) {
      scale.value = 1;
      x.value = 0;
      y.value = 0;
    } else {
      scale.value = clamp(scale.value, MIN_SCALE, MAX_SCALE);
      const panTo = clampPan(x.value, y.value, scale.value, box);
      x.value = panTo.x;
      y.value = panTo.y;
    }
    pinch = null;
    mode = "idle";
    active.value = false;
    if (event.touches.length === 1) {
      const t = event.touches[0];
      pan = { x: t.clientX, y: t.clientY, ox: x.value, oy: y.value };
      mode = "pan";
      active.value = true;
    }
  }

  function toggleZoom(event, clientX, clientY) {
    const box = stageBox(event.currentTarget);
    if (scale.value > SNAP) {
      scale.value = 1;
      x.value = 0;
      y.value = 0;
      return;
    }
    zoomAround(2.6, clientX - box.left, clientY - box.top, box);
  }

  function videoControlsTouch(event) {
    const target = event.target;
    if (!(target instanceof HTMLVideoElement)) return false;
    const touch = event.touches?.[0] || event.changedTouches?.[0];
    if (!touch) return false;
    const rect = target.getBoundingClientRect();
    return touch.clientY >= rect.bottom - 64;
  }

  function pinchAllowed() {
    return !allowPinch || allowPinch();
  }

  function onTouchStart(event) {
    if (isLocked?.()) return;
    if (videoControlsTouch(event)) {
      mode = "exempt";
      return;
    }
    stopCoast();
    onSlideGrab?.();
    if (event.touches.length >= 2) {
      if (!pinchAllowed()) return;
      event.preventDefault();
      beginPinch(event);
      return;
    }
    if (event.touches.length !== 1) return;
    const t = event.touches[0];
    pan = { x: t.clientX, y: t.clientY, ox: x.value, oy: y.value };
    tapOrigin = { x: t.clientX, y: t.clientY, at: performance.now() };
    slideOriginX = t.clientX - slideX.value;
    lastMoveX = t.clientX;
    lastMoveY = t.clientY;
    lastMoveT = performance.now();
    velocityX = 0;
    velocityY = 0;
    moved = false;
    lift = null;
    if (scale.value > SNAP) {
      mode = "pan";
      active.value = true;
    }
  }

  function onTouchMove(event) {
    if (isLocked?.()) return;
    if (mode === "exempt") return;
    if (event.touches.length >= 2 && !pinchAllowed()) return;
    event.preventDefault();
    if (event.touches.length >= 2) {
      if (mode !== "pinch") beginPinch(event);
      else applyPinch(event);
      return;
    }
    if (mode === "pinch") {
      endPinch(event);
    }
    if (event.touches.length !== 1 || !pan) return;
    const t = event.touches[0];
    const dx = t.clientX - pan.x;
    const dy = t.clientY - pan.y;
    if (Math.hypot(dx, dy) > TAP_PX) moved = true;
    const now = performance.now();
    const dt = now - lastMoveT;
    if (dt > 0) {
      velocityX = (t.clientX - lastMoveX) / dt;
      velocityY = (t.clientY - lastMoveY) / dt;
    }
    lastMoveX = t.clientX;
    lastMoveY = t.clientY;
    lastMoveT = now;

    if (scale.value > SNAP) {
      event.preventDefault();
      const box = stageBox(event.currentTarget);
      const panTo = clampPan(pan.ox + dx, pan.oy + dy, scale.value, box);
      x.value = panTo.x;
      y.value = panTo.y;
      return;
    }

    if (!sliding.value && !lift) {
      if (Math.hypot(dx, dy) < SLIDE_LOCK_PX) return;
      if (Math.abs(dy) > Math.abs(dx) * 1.1) {
        lift = dy < 0 ? "up" : "down";
        moved = true;
        window.clearTimeout(tapTimer);
        tapTimer = 0;
      } else {
        sliding.value = true;
        mode = "slide";
        active.value = true;
        window.clearTimeout(tapTimer);
        tapTimer = 0;
        onSlideStart?.(dx < 0 ? "next" : "prev");
        slideOriginX = t.clientX - slideX.value;
      }
    }
    if (lift === "down" && canDismiss?.()) {
      event.preventDefault();
      onDismissMove?.(Math.max(0, dy));
      return;
    }
    if (lift || !sliding.value) return;
    event.preventDefault();
    const box = stageBox(event.currentTarget);
    pullSlide(t.clientX - slideOriginX, box.w);
  }

  function onTouchEnd(event) {
    if (mode === "exempt") {
      mode = "idle";
      return;
    }
    if (isLocked?.()) return;
    if (mode === "pinch") {
      endPinch(event);
      return;
    }
    if (event.touches.length > 0) {
      if (event.touches.length === 1) {
        const t = event.touches[0];
        pan = { x: t.clientX, y: t.clientY, ox: x.value, oy: y.value };
      }
      return;
    }
    const ended = event.changedTouches[0];
    const origin = tapOrigin;
    const wasSliding = sliding.value;
    const liftKind = lift;
    const dxSlide = slideX.value;
    const vx = (performance.now() - lastMoveT) > 80 ? 0 : velocityX;
    const vy = (performance.now() - lastMoveT) > 80 ? 0 : velocityY;
    const width = stageBox(event.currentTarget).w;
    mode = "idle";
    pan = null;
    tapOrigin = null;
    lift = null;
    active.value = false;
    sliding.value = false;
    if (scale.value > SNAP && moved && !liftKind && !wasSliding) {
      startPanCoast(event.currentTarget, vx, vy);
      return;
    }
    if (liftKind && ended && origin) {
      const dy = ended.clientY - origin.y;
      if (liftKind === "up" && dy < -36) onSwipeUp?.();
      else if (liftKind === "down" && canDismiss?.()) onSwipeDown?.({ dy: Math.max(0, dy), vy });
      else if (liftKind === "down" && dy > 36) onSwipeDown?.({ dy, vy });
      return;
    }
    if (wasSliding) {
      onSlideEnd?.({ dx: dxSlide, vx, width, cancelled: false });
      return;
    }
    if (!ended || !origin) return;
    const dx = ended.clientX - origin.x;
    const dy = ended.clientY - origin.y;
    const dist = Math.hypot(dx, dy);
    if (!moved && dist < TAP_PX) {
      const now = performance.now();
      if (now - lastTapAt < DOUBLE_MS && Math.hypot(ended.clientX - lastTapX, ended.clientY - lastTapY) < 40) {
        window.clearTimeout(tapTimer);
        tapTimer = 0;
        lastTapAt = 0;
        toggleZoom(event, ended.clientX, ended.clientY);
        return;
      }
      lastTapAt = now;
      lastTapX = ended.clientX;
      lastTapY = ended.clientY;
      window.clearTimeout(tapTimer);
      tapTimer = window.setTimeout(() => {
        tapTimer = 0;
        onTap?.();
      }, DOUBLE_MS);
    }
  }

  onBeforeUnmount(() => {
    stopCoast();
    window.clearTimeout(tapTimer);
  });

  return {
    scale,
    zoomed,
    active,
    sliding,
    slideX,
    imgStyle,
    reset,
    resetZoom,
    resetSlide,
    onTouchStart,
    onTouchMove,
    onTouchEnd,
  };
}
