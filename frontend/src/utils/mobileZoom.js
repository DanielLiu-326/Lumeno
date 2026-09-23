import { setUiScaleInflation } from "./uiScale.js";

const VIEWPORT_BASE = "width=device-width, initial-scale=1, viewport-fit=cover, shrink-to-fit=no";
const VIEWPORT_NO_ZOOM = `${VIEWPORT_BASE}, maximum-scale=1, user-scalable=no`;

let locked = false;

function isMobile() {
  return window.innerWidth < 768 || screen.width < 768;
}

function isSafari() {
  return document.documentElement.getAttribute("data-browser") === "safari";
}

function isTextField(el) {
  return (
    el instanceof HTMLElement &&
    el.matches(
      "input:not([type=checkbox]):not([type=radio]):not([type=range]):not([type=file]):not([type=hidden]):not([type=button]):not([type=submit]):not([type=reset]):not([type=color]), textarea, select, [contenteditable='true']",
    )
  );
}

function restViewport() {
  return isSafari() ? VIEWPORT_NO_ZOOM : VIEWPORT_BASE;
}

function minFontInflation() {
  const probe = document.createElement("div");
  probe.style.cssText =
    "position:absolute;left:0;top:0;font-size:12px;line-height:1;visibility:hidden;pointer-events:none;";
  probe.textContent = "A";
  document.documentElement.appendChild(probe);
  const actual = parseFloat(getComputedStyle(probe).fontSize) || 12;
  probe.remove();
  return actual;
}

export function lockMobileZoom() {
  if (locked || !isMobile() || isSafari()) return;

  const meta = document.querySelector('meta[name="viewport"]');
  const scale = window.visualViewport?.scale || 1;
  const inflated = minFontInflation();

  if (scale > 1.02 && meta) {
    locked = true;
    meta.setAttribute("content", `${VIEWPORT_BASE}, maximum-scale=1.0001`);
    requestAnimationFrame(() => {
      meta.setAttribute("content", VIEWPORT_BASE);
    });
    return;
  }

  if (inflated > 13) {
    locked = true;
    setUiScaleInflation(Math.max(0.72, 12 / inflated));
  }
}

export function preventInputPageZoom() {
  const meta = document.querySelector('meta[name="viewport"]');
  if (!meta) return;

  document.addEventListener("focusin", (event) => {
    if (!isTextField(event.target)) return;
    meta.setAttribute("content", VIEWPORT_NO_ZOOM);
  });

  document.addEventListener("focusout", (event) => {
    if (!isTextField(event.target)) return;
    requestAnimationFrame(() => {
      if (isTextField(document.activeElement)) return;
      meta.setAttribute("content", restViewport());
    });
  });
}
