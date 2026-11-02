// Storage report state: the report payload, its polling and the two tasks that
// change it. `useAlbumApp` keeps the app-wide state; this is the one panel that
// owns a request loop of its own.
import { ref } from "vue";

import { t } from "../i18n/index.js";

const POLL_MS = 2500;
const CLEANUP_POLL_MS = 3000;
// A cleanup drops the cached report when it finishes, so keep looking until the
// follow-up scan shows up (or give up after a few minutes).
const CLEANUP_POLL_LIMIT = 60;

/**
 * @param {object} context
 * @param {(url: string, options?: object) => Promise<any>} context.apiFetch
 * @param {() => boolean} context.isLoggedIn
 * @param {(message: string, kind: "error"|"success") => void} context.showMessage
 * @param {() => Promise<void>} context.refreshTasks
 * @param {() => void} context.ensureTaskPolling
 */
export function useStorageManager({ apiFetch, isLoggedIn, showMessage, refreshTasks, ensureTaskPolling }) {
  const report = ref(null);
  const loading = ref(false);
  const error = ref("");
  const stale = ref(false);
  const scanning = ref(false);
  // A cleanup runs as a task; the panel needs to show that something is
  // happening before the next 3 s poll lands.
  const cleaning = ref(false);
  // Admin view: `owner` is the account whose report is on screen ("" = self).
  const owner = ref("");
  const members = ref([]);
  const sharedCache = ref([]);
  const membersLoading = ref(false);
  const membersError = ref("");

  let seq = 0;
  let pollTimer = null;
  let cleanupTimer = null;
  let membersTimer = null;
  let membersSeq = 0;

  function stopPolling() {
    if (pollTimer) {
      clearTimeout(pollTimer);
      pollTimer = null;
    }
    if (cleanupTimer) {
      clearInterval(cleanupTimer);
      cleanupTimer = null;
    }
    if (membersTimer) {
      clearTimeout(membersTimer);
      membersTimer = null;
    }
  }

  function watchCleanup() {
    if (cleanupTimer) clearInterval(cleanupTimer);
    let attempts = 0;
    cleanupTimer = setInterval(async () => {
      attempts += 1;
      await refresh();
      // The scan that follows a cleanup sets `scanning`; a key-level cleanup
      // just drops the report, so stop after the report changes or the
      // attempts run out.
      if (scanning.value || !report.value || attempts >= CLEANUP_POLL_LIMIT) {
        clearInterval(cleanupTimer);
        cleanupTimer = null;
        cleaning.value = false;
      }
    }, CLEANUP_POLL_MS);
  }

  function schedulePoll() {
    if (pollTimer) return;
    pollTimer = setTimeout(async () => {
      pollTimer = null;
      await refresh();
    }, POLL_MS);
  }

  async function refresh() {
    if (!isLoggedIn()) return;
    const request = ++seq;
    loading.value = true;
    error.value = "";
    try {
      const query = owner.value ? `?owner=${encodeURIComponent(owner.value)}` : "";
      const payload = await apiFetch(`/api/storage/report${query}`, { cache: "no-store" });
      if (request !== seq) return;
      const data = payload.data || {};
      report.value = data.report || null;
      stale.value = Boolean(data.stale);
      scanning.value = Boolean(data.scanning);
      if (scanning.value) {
        if (owner.value) scheduleMembersPoll();
        else schedulePoll();
      }
    } catch (requestError) {
      if (request !== seq) return;
      error.value = requestError.message || t("storage.loadError");
    } finally {
      if (request === seq) loading.value = false;
    }
  }

  async function scan(target = owner.value) {
    try {
      const query = target
        ? `?owner=${encodeURIComponent(target)}`
        : "";
      await apiFetch(`/api/storage/scan${query}`, { method: "POST" });
      scanning.value = true;
      void refreshTasks();
      ensureTaskPolling();
      await refresh();
      void loadMembers();
    } catch (scanError) {
      showMessage(scanError.message, "error");
    }
  }

  /// Queue one scan per member, bounded by the server.
  async function scanAll() {
    try {
      await apiFetch("/api/storage/scan?all=1", { method: "POST" });
      scanning.value = true;
      void refreshTasks();
      ensureTaskPolling();
      await loadMembers();
    } catch (scanError) {
      showMessage(scanError.message, "error");
    }
  }

  async function cleanup(spec) {
    cleaning.value = true;
    try {
      const body = owner.value ? { ...spec, owner: owner.value } : spec;
      const payload = await apiFetch("/api/storage/cleanup", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      // The task is queued: do not make the dialog wait for the task list.
      void refreshTasks();
      ensureTaskPolling();
      watchCleanup();
      return payload.data || null;
    } catch (cleanupError) {
      cleaning.value = false;
      showMessage(cleanupError.message, "error");
      return null;
    }
  }

  async function loadMembers() {
    if (!isLoggedIn()) return;
    const request = ++membersSeq;
    membersLoading.value = true;
    membersError.value = "";
    try {
      const payload = await apiFetch("/api/storage/members", { cache: "no-store" });
      if (request !== membersSeq) return;
      const data = payload.data || {};
      members.value = data.members || [];
      sharedCache.value = data.shared_cache || [];
      if (members.value.some((row) => row.scanning)) scheduleMembersPoll();
    } catch (membersRequestError) {
      if (request !== membersSeq) return;
      members.value = [];
      membersError.value = membersRequestError.message || t("storage.membersLoadError");
    } finally {
      if (request === membersSeq) membersLoading.value = false;
    }
  }

  function scheduleMembersPoll() {
    if (membersTimer) return;
    membersTimer = setTimeout(async () => {
      membersTimer = null;
      await loadMembers();
    }, CLEANUP_POLL_MS);
  }

  /// Switch the panel to another account's report ("" = back to self).
  async function viewOwner(next) {
    const target = String(next || "");
    if (target === owner.value) return;
    owner.value = target;
    report.value = null;
    stale.value = true;
    scanning.value = false;
    await refresh();
    if (!report.value) await loadMembers();
  }

  /// Forget the report and the timers (logout, account switch).
  function reset() {
    report.value = null;
    error.value = "";
    stale.value = false;
    scanning.value = false;
    cleaning.value = false;
    owner.value = "";
    members.value = [];
    sharedCache.value = [];
    stopPolling();
  }

  return {
    report,
    loading,
    error,
    stale,
    scanning,
    cleaning,
    owner,
    members,
    sharedCache,
    membersLoading,
    membersError,
    refresh,
    loadMembers,
    viewOwner,
    scanAll,
    scan,
    cleanup,
    reset,
    stopPolling,
  };
}
