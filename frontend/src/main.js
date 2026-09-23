import { createApp } from "vue";
import DesktopApp from "./App.vue";
import MobileApp from "./mobile/App.vue";
import { createMobileRouter } from "./mobile/router.js";
import "./styles.css";
import { isMobileShell, applyShellAttribute } from "./utils/device.js";
import { scheduleSafeAreaSync, syncSafeAreaInsets } from "./utils/safeArea.js";
import { lockMobileZoom, preventInputPageZoom } from "./utils/mobileZoom.js";
import { initUiScale } from "./utils/uiScale.js";

initUiScale();
lockMobileZoom();
preventInputPageZoom();
syncSafeAreaInsets();
window.addEventListener("pageshow", () => {
  lockMobileZoom();
  scheduleSafeAreaSync();
});
window.addEventListener("resize", scheduleSafeAreaSync);
window.addEventListener("orientationchange", scheduleSafeAreaSync);
window.visualViewport?.addEventListener("resize", scheduleSafeAreaSync);

const mobile = isMobileShell();
applyShellAttribute(mobile);

if (mobile) {
  installMobilePress();
  const app = createApp(MobileApp);
  app.use(createMobileRouter());
  app.mount("#app");
} else {
  createApp(DesktopApp).mount("#app");
}

const MOBILE_PRESSABLE = [
  "button",
  "a",
  ".mobile-row",
  ".mobile-sheet-action",
  ".mobile-sheet-cancel",
  ".mobile-quick",
  ".mobile-chip-btn",
  ".mobile-logout",
  ".mobile-profile",
  ".mobile-album-card",
  ".mobile-album-chip",
  ".file-path-nav",
  ".mobile-sheet-segment-btn",
  ".mobile-sort-dir-chip",
  "[data-entry-card]",
].join(", ");

const MOBILE_PRESS_SKIP = ":disabled, .mobile-sheet-mask, .mobile-tabbar-item, [data-press='off']";

function installMobilePress() {
  let pressed = null;
  let originX = 0;
  let originY = 0;

  function clearPressed() {
    pressed?.classList.remove("is-pressed");
    pressed = null;
  }

  document.addEventListener("pointerdown", (event) => {
    if (event.button != null && event.button > 0) return;
    const node = event.target;
    if (!(node instanceof Element)) return;
    const el = node.closest(MOBILE_PRESSABLE);
    if (!el || el.matches(MOBILE_PRESS_SKIP) || el.closest(".mobile-tabbar, [data-press='off']")) return;
    clearPressed();
    pressed = el;
    originX = event.clientX;
    originY = event.clientY;
    el.classList.add("is-pressed");
  });

  document.addEventListener("pointermove", (event) => {
    if (!pressed) return;
    if (Math.abs(event.clientX - originX) > 12 || Math.abs(event.clientY - originY) > 12) {
      clearPressed();
    }
  });

  document.addEventListener("pointerup", clearPressed);
  document.addEventListener("pointercancel", clearPressed);
}

function signalReady() {
  window.dispatchEvent(new Event("lumeno:ready"));
}

const fontsReady = document.fonts?.ready;
if (fontsReady && typeof fontsReady.then === "function") {
  fontsReady.then(() => requestAnimationFrame(() => requestAnimationFrame(signalReady)));
} else {
  requestAnimationFrame(() => requestAnimationFrame(signalReady));
}

if (import.meta.env.PROD && "serviceWorker" in navigator) {
  const host = window.location.hostname;
  const isSecure =
    window.location.protocol === "https:" ||
    host === "localhost" ||
    host === "127.0.0.1" ||
    host === "[::1]";
  if (isSecure) {
    window.addEventListener("load", () => {
      navigator.serviceWorker.register("/sw.js", { updateViaCache: "none" }).catch(() => {});
    });
  }
}
