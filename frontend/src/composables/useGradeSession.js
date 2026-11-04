import { computed, onBeforeUnmount, ref } from "vue";
import { httpFetch } from "../utils/http";

const PATCH_DELAY = 90;
const JSON_HEADERS = { "Content-Type": "application/json" };

async function postJson(url, body) {
  const res = await httpFetch(url, {
    method: "POST",
    headers: JSON_HEADERS,
    body: JSON.stringify(body),
  });
  const data = await res.json().catch(() => ({}));
  return { res, data };
}

/**
 * One grade document, shared by the mobile editor. The server owns the
 * document: every patch returns the new `rev` and the re-rendered preview, and
 * writes the `.lumeno.json` sidecar. Local node params are updated first so the
 * sliders never lag behind the finger.
 */
export function useGradeSession() {
  const path = ref("");
  const rev = ref(0);
  const raw = ref(false);
  const nodes = ref([]);
  const preview = ref("");
  const before = ref("");
  const loading = ref(false);
  const error = ref("");
  const saved = ref(true);
  const canWrite = ref(true);

  const queue = new Map();
  const editing = new Set();
  let sendTimer = 0;
  let inflight = false;
  let editSnapshot = null;
  const undoStack = [];
  const redoStack = [];
  const historyVersion = ref(0);

  const byId = computed(() => new Map(nodes.value.map((node) => [node.id, node])));

  function node(id) {
    return byId.value.get(id) || null;
  }

  async function open(photoPath) {
    path.value = photoPath;
    loading.value = true;
    error.value = "";
    queue.clear();
    undoStack.length = 0;
    redoStack.length = 0;
    try {
      const { res, data } = await postJson("/api/grade/open", { path: photoPath });
      if (!res.ok) throw new Error(data.message || "open failed");
      rev.value = Number(data.rev) || 0;
      raw.value = Boolean(data.raw);
      nodes.value = Array.isArray(data.nodes) ? data.nodes : [];
      canWrite.value = data.writable !== false;
      preview.value = data.preview || "";
      before.value = preview.value;
      if (!preview.value) await refresh();
      saved.value = true;
    } catch (err) {
      error.value = err.message || String(err);
    } finally {
      loading.value = false;
    }
  }

  async function refresh() {
    if (!rev.value) return;
    try {
      const { res, data } = await postJson("/api/grade/preview", { rev: rev.value });
      if (!res.ok) return;
      if (data.preview) preview.value = data.preview;
      if (!before.value) before.value = data.preview || "";
    } catch {
      /* the next patch retries */
    }
  }

  function markDirty(id) {
    saved.value = false;
    queue.set(id, node(id));
    schedule();
  }

  function schedule() {
    clearTimeout(sendTimer);
    sendTimer = window.setTimeout(flush, PATCH_DELAY);
  }

  async function flush() {
    if (inflight || !queue.size || !rev.value) return;
    const [id] = queue.entries().next().value;
    queue.delete(id);
    const current = node(id);
    if (!current) return;
    inflight = true;
    try {
      const { res, data } = await postJson("/api/grade/node", {
        rev: rev.value,
        id,
        enabled: current.enabled !== false,
        params: current.params,
      });
      const reason = data?.data?.reason;
      if (res.status === 409 || reason === "not-open" || reason === "stale") {
        await reopen();
        return;
      }
      if (data?.superseded) return;
      if (!res.ok) throw new Error(data.message || "patch failed");
      rev.value = Number(data.rev) || rev.value;
      if (data.node) {
        const index = nodes.value.findIndex((entry) => entry.id === id);
        if (index >= 0) {
          // The response describes the params of the request that was sent a
          // moment ago. While the finger is still down (or more input is
          // queued) the local params are newer: keeping them stops the thumb
          // from jumping back to the stale value.
          const local = nodes.value[index];
          const stale = editing.has(id) || queue.has(id);
          nodes.value.splice(index, 1, {
            ...data.node,
            enabled: stale ? local.enabled : data.node.enabled,
            params: stale ? local.params : data.node.params,
          });
        }
      }
      if (data.preview) preview.value = data.preview;
      error.value = "";
      saved.value = queue.size === 0;
    } catch (err) {
      error.value = err.message || String(err);
      saved.value = false;
    } finally {
      inflight = false;
      if (queue.size) schedule();
    }
  }

  /** Re-open after the server dropped the session, then replay local edits. */
  async function reopen() {
    const local = new Map();
    for (const entry of nodes.value) local.set(entry.id, entry);
    const photo = path.value;
    const openRev = rev.value;
    await open(photo);
    if (!rev.value || !openRev) return;
    for (const [id, entry] of local) {
      const fresh = node(id);
      if (!fresh) continue;
      if (JSON.stringify(fresh.params) === JSON.stringify(entry.params)) continue;
      fresh.params = entry.params;
      fresh.enabled = entry.enabled;
      queue.set(id, fresh);
    }
    if (queue.size) schedule();
  }

  function setParam(id, key, value) {
    const target = node(id);
    if (!target) return;
    target.params = { ...target.params, [key]: value };
    target.enabled = true;
    markDirty(id);
  }

  function setChoice(id, key, value) {
    setParam(id, key, value);
  }

  function beginEdit(id) {
    const target = node(id);
    if (!target) return;
    editing.add(id);
    editSnapshot = { id, params: JSON.stringify(target.params) };
  }

  function endEdit() {
    if (!editSnapshot) return;
    editing.delete(editSnapshot.id);
    const target = node(editSnapshot.id);
    const after = target ? JSON.stringify(target.params) : "";
    if (target && after !== editSnapshot.params) {
      undoStack.push({ id: editSnapshot.id, before: editSnapshot.params, after });
      redoStack.length = 0;
      if (undoStack.length > 40) undoStack.shift();
      historyVersion.value += 1;
    }
    editSnapshot = null;
  }

  function apply(id, params) {
    const target = node(id);
    if (!target) return;
    target.params = JSON.parse(params);
    target.enabled = true;
    markDirty(id);
  }

  function undo() {
    const step = undoStack.pop();
    if (!step) return;
    redoStack.push(step);
    historyVersion.value += 1;
    apply(step.id, step.before);
  }

  function redo() {
    const step = redoStack.pop();
    if (!step) return;
    undoStack.push(step);
    historyVersion.value += 1;
    apply(step.id, step.after);
  }

  function close() {
    clearTimeout(sendTimer);
    queue.clear();
    nodes.value = [];
    preview.value = "";
    before.value = "";
    rev.value = 0;
  }

  onBeforeUnmount(close);

  return {
    path,
    rev,
    raw,
    nodes,
    preview,
    before,
    loading,
    error,
    saved,
    canWrite,
    node,
    open,
    refresh,
    setParam,
    setChoice,
    beginEdit,
    endEdit,
    undo,
    redo,
    canUndo: computed(() => historyVersion.value >= 0 && undoStack.length > 0),
    canRedo: computed(() => historyVersion.value >= 0 && redoStack.length > 0),
  };
}
