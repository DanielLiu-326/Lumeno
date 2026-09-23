import { onBeforeUnmount, unref, watch } from "vue";

let ignorePop = 0;
const stack = [];
let listening = false;

function onPopState() {
  if (ignorePop > 0) {
    ignorePop -= 1;
    return;
  }
  const top = stack.pop();
  if (!top) return;
  top.pushed = false;
  top.closeFn();
}

function ensureListen() {
  if (listening) return;
  window.addEventListener("popstate", onPopState);
  listening = true;
}

function whenHistoryIdle(fn) {
  if (ignorePop <= 0) {
    fn();
    return;
  }
  const started = Date.now();
  const wait = () => {
    if (ignorePop <= 0 || Date.now() - started > 400) {
      fn();
      return;
    }
    window.setTimeout(wait, 0);
  };
  window.setTimeout(wait, 0);
}

export function discardOverlayStack() {
  const entries = stack.splice(0, stack.length);
  for (const entry of entries) {
    entry.pushed = false;
    entry.closeFn();
  }
}

export function currentHistoryIsOverlay() {
  return Boolean(history.state?.lumenoOverlay);
}

export function useOverlayHistory(openRef, closeFn) {
  const entry = { closeFn, pushed: false };

  watch(openRef, (open) => {
    entry.closeFn = closeFn;
    if (open) {
      if (entry.pushed) return;
      ensureListen();
      whenHistoryIdle(() => {
        if (entry.pushed || !unref(openRef)) return;
        history.pushState({ lumenoOverlay: true }, "");
        entry.pushed = true;
        stack.push(entry);
      });
      return;
    }
    if (!entry.pushed) return;
    const idx = stack.lastIndexOf(entry);
    if (idx >= 0) stack.splice(idx, 1);
    entry.pushed = false;
    ignorePop += 1;
    history.back();
  });

  onBeforeUnmount(() => {
    if (!entry.pushed) return;
    const idx = stack.lastIndexOf(entry);
    if (idx >= 0) stack.splice(idx, 1);
    entry.pushed = false;
    ignorePop += 1;
    history.back();
  });
}
