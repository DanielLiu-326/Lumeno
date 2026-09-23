const HOME_INDICATOR = 34;
let lastBottomPx = 0;
let settleTimer = 0;

function cssPx(n) {
  const value = Number.isFinite(n) && n > 0 ? n : 0;
  return `${value}px`;
}

function readPx(value) {
  const n = parseFloat(value);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

function measureInsets() {
  const probe = document.createElement("div");
  probe.setAttribute("aria-hidden", "true");
  probe.style.cssText = [
    "position:fixed",
    "inset:0",
    "width:auto",
    "height:auto",
    "pointer-events:none",
    "visibility:hidden",
    "padding-top:env(safe-area-inset-top,0px)",
    "padding-right:env(safe-area-inset-right,0px)",
    "padding-bottom:env(safe-area-inset-bottom,0px)",
    "padding-left:env(safe-area-inset-left,0px)",
  ].join(";");
  document.documentElement.appendChild(probe);
  const cs = getComputedStyle(probe);
  const insets = {
    top: readPx(cs.paddingTop),
    right: readPx(cs.paddingRight),
    bottom: readPx(cs.paddingBottom),
    left: readPx(cs.paddingLeft),
  };
  probe.remove();
  return insets;
}

function applyInsets() {
  const root = document.documentElement;
  const insets = measureInsets();
  let bottom = insets.bottom;
  const portrait = window.innerHeight >= window.innerWidth;
  if (portrait && insets.top >= 44 && bottom < 8) {
    bottom = lastBottomPx >= 8 ? lastBottomPx : HOME_INDICATOR;
  }
  if (bottom >= 8) lastBottomPx = bottom;

  root.style.setProperty("--safe-top-js", cssPx(insets.top));
  root.style.setProperty("--safe-right-js", cssPx(insets.right));
  root.style.setProperty("--safe-bottom-js", cssPx(bottom));
  root.style.setProperty("--safe-left-js", cssPx(insets.left));
  root.style.removeProperty("--safe-top");
  root.style.removeProperty("--safe-right");
  root.style.removeProperty("--safe-bottom");
  root.style.removeProperty("--safe-left");
}

export function syncSafeAreaInsets() {
  applyInsets();
}

export function scheduleSafeAreaSync() {
  applyInsets();
  window.clearTimeout(settleTimer);
  settleTimer = window.setTimeout(applyInsets, 320);
  requestAnimationFrame(applyInsets);
}
