import { nextTick } from "vue";

const FADE_MS = 450;

/** Crossfade the page when the theme class changes. Skip on first paint and reduced motion. */
export function withThemeTransition(apply, { animate = false } = {}) {
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!animate || reduced) {
    apply();
    return;
  }
  const root = document.documentElement;
  if (typeof document.startViewTransition !== "function") {
    root.classList.add("theme-fade");
    apply();
    window.setTimeout(() => root.classList.remove("theme-fade"), FADE_MS);
    return;
  }
  root.classList.add("theme-vt");
  let transition;
  try {
    transition = document.startViewTransition(async () => {
      apply();
      await nextTick();
    });
  } catch {
    root.classList.remove("theme-vt");
    apply();
    return;
  }
  transition.finished.finally(() => {
    root.classList.remove("theme-vt");
  });
}
