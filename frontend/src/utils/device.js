const DESKTOP_QUERY = "?desktop=1";
const MOBILE_QUERY = "?mobile=1";

export function isMobileShell() {
  try {
    const params = new URLSearchParams(window.location.search);
    if (params.has("desktop") && params.get("desktop") !== "0") {
      sessionStorage.setItem("lumeno-shell", "desktop");
    } else if (params.has("mobile") && params.get("mobile") !== "0") {
      sessionStorage.setItem("lumeno-shell", "mobile");
    }
    const forced = sessionStorage.getItem("lumeno-shell");
    if (forced === "desktop") return false;
    if (forced === "mobile") return true;
  } catch {
    /* ignore */
  }

  const standalone = window.matchMedia("(display-mode: standalone)").matches
    || window.navigator.standalone === true;
  const coarse = window.matchMedia("(hover: none) and (pointer: coarse)").matches;
  const narrow = window.innerWidth < 768 || window.screen.width < 768;
  return narrow || (standalone && coarse);
}

export function applyShellAttribute(mobile) {
  document.documentElement.setAttribute("data-shell", mobile ? "mobile" : "desktop");
}

export { DESKTOP_QUERY, MOBILE_QUERY };
