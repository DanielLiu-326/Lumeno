/**
 * Image load stack + worker pool.
 *
 * Architecture:
 *   - Callers push tasks onto a LIFO stack via enqueue().
 *   - A pool of up to MAX_WORKERS async functions continuously pop and process
 *     tasks.  Each worker independently loops: pop → preload → pop → preload…
 *     so a finished slot is immediately refilled from the top of the stack.
 *   - Cancellation via AbortSignal: synchronously splices the task out of the
 *     stack so it is never processed.
 *   - Preloading uses a hidden Image element so the URL lands in the browser's
 *     memory cache before the visible <img> element renders it.
 *   - loadedUrls is module-level so it survives component remounts.
 */

const MAX_WORKERS = 6;

/** URLs successfully preloaded in this page session. */
export const loadedUrls = new Set();

/** LIFO stack – push to end, pop from end (newest = highest priority). */
const stack = [];
let activeWorkers = 0;

/** Start new workers up to MAX_WORKERS as long as the stack has tasks. */
function trySpawnWorkers() {
  while (activeWorkers < MAX_WORKERS && stack.length > 0) {
    activeWorkers++;
    runWorker(); // intentionally not awaited
  }
}

/** A single worker loop: pop → process → pop → … until the stack is empty. */
async function runWorker() {
  while (stack.length > 0) {
    const task = stack.pop(); // LIFO
    await processTask(task);
  }
  activeWorkers--;
}

/**
 * Preload one image via a hidden Image element.
 * Resolves the task's settle function when done (success or abort/error).
 * Returns a Promise that resolves when the worker slot is free for the next task.
 */
function processTask({ url, signal, settle }) {
  return new Promise((workerDone) => {
    if (signal?.aborted) {
      settle(new DOMException("Aborted", "AbortError"));
      workerDone();
      return;
    }
    // Already cached while waiting in stack.
    if (loadedUrls.has(url)) {
      settle(null);
      workerDone();
      return;
    }

    const img = new Image();
    img.fetchPriority = "low";

    const finish = (err) => {
      img.onload = img.onerror = null;
      settle(err ?? null);
      workerDone();
    };
    const onAbort = () => { img.src = ""; finish(new DOMException("Aborted", "AbortError")); };
    signal?.addEventListener("abort", onAbort, { once: true });

    img.onload = () => {
      signal?.removeEventListener("abort", onAbort);
      loadedUrls.add(url);
      finish(null);
    };
    img.onerror = () => {
      signal?.removeEventListener("abort", onAbort);
      finish(new Error("Image failed to load"));
    };
    img.src = url;
  });
}

/**
 * Push a URL onto the LIFO load stack.
 * @param {string}            url     Thumbnail URL to preload.
 * @param {AbortSignal|undefined} signal  Cancel the load (removes from stack).
 * @returns {Promise<void>}  Resolves when the image is in loadedUrls.
 *                           Rejects with AbortError if cancelled.
 */
export function enqueue(url, signal) {
  if (loadedUrls.has(url)) return Promise.resolve();
  if (signal?.aborted) return Promise.reject(new DOMException("Aborted", "AbortError"));

  return new Promise((resolve, reject) => {
    let settled = false;
    const settle = (err) => {
      if (settled) return;
      settled = true;
      err ? reject(err) : resolve();
    };

    const task = { url, signal, settle };
    stack.push(task);

    // Cancel: remove from stack and reject immediately.
    signal?.addEventListener(
      "abort",
      () => {
        const i = stack.indexOf(task);
        if (i >= 0) stack.splice(i, 1);
        settle(new DOMException("Aborted", "AbortError"));
      },
      { once: true },
    );

    trySpawnWorkers();
  });
}
