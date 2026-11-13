import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { formatSize, formatTime, progressPercent, statusLabel, statusTone } from "../utils/format";
import { locale, setLocale, t } from "../i18n/index.js";
import { httpJson, setUnauthorizedHandler } from "../utils/http";
import { compareRelativePaths, encodePath, joinPath, prettyPath, normalizeBrowsePath } from "../utils/path";
import { compileAlbumFilterClauses, loadFilterItemsFromStorage, partitionFilterClauses } from "../utils/filterDefs.js";
import { useStorageManager } from "./useStorageManager.js";
import { defaultWallpaper, normalizeWallpaper } from "../utils/wallpaper.js";
import { defaultAccent, loadCachedAccent, normalizeAccent } from "../utils/accent.js";
import { downloadFileByUrl } from "../utils/downloadFile.js";
import { setVideoEncodeOrder } from "../utils/videoEncode.js";

const UPLOAD_CHUNK_SIZE = 4 * 1024 * 1024;
const DEFAULT_PAGE_SIZE = 100;

function clampPageSize(value) {
    const n = Number(value);
    if (!Number.isFinite(n)) return DEFAULT_PAGE_SIZE;
    return Math.min(500, Math.max(10, Math.round(n)));
}

export function useAlbumApp() {
    const loginForm = reactive({
        username: "",
        passphrase: "",
        keepLogin: true,
    });

    const busy = reactive({
        login: false,
        list: false,
        upload: false,
        tasks: false,
        albums: false,
    });

    const loadingMore = reactive({
        entries: false,
        photos: false,
    });

    const message = reactive({
        kind: "info",
        text: "",
        sequence: 0,
    });

    const currentPath = ref("");
    const pathHistory = ref([""]);
    const pathHistoryIndex = ref(0);
    const entries = ref([]);
    const entryOffset = ref(0);
    const entryTotal = ref(0);
    const viewMode = ref("list");
    const tasks = ref([]);
    const selectedPaths = ref([]);
    const loggedIn = ref(false);
    const clipboard = ref([]);
    const clipboardMode = ref("copy");
    const albums = ref([]);
    const currentAlbum = ref("");
    const uploadCancelled = ref(false);
    const albumViewMode = ref("photos");
    const albumPhotos = ref([]);
    const albumPhotoOffset = ref(0);
    const albumPhotoTotal = ref(0);
    const albumBursts = ref({ state: "ready", algorithm: "burst-v1", stack_count: 0 });
    const collapseBursts = ref(readCollapseBurstsPreference());
    const burstThreshold = ref(10);
    const burstAlgorithm = ref("dct");
    const burstEmbeddingThreshold = ref(80);
    let albumPhotosFillQueued = false;
    let burstRefreshTimer = null;
    let burstRefreshAttempts = 0;
    const albumDuplicateGroups = ref([]);
    const albumDuplicateTotal = ref(0);
    const aiSimilarResults = ref([]);
    const aiSimilarQuery = ref(null);
    const aiSimilarLoading = ref(false);
    const mobileSearchPhotos = ref([]);
    const mobileSearchLoading = ref(false);
    const mobileSearchText = ref("");
    let mobileSearchFetchId = 0;
    const albumAvailableTags = ref([]);
    const albumAvailableCameras = ref([]);
    const albumAvailableLenses = ref([]);
    const albumAvailableFormats = ref([]);
    const albumFilters = reactive({
        /** Logical filter clauses sent to POST /api/albums/photos */
        clauses: [],
    });
    let albumPhotosFetchId = 0;
    const albumSort = ref([]);
    const dateHistogram = ref([]);
    const lastSelectedIndex = ref(null);
    const selectAllActive = ref(false);
    const sharedAlbums = ref([]);
    const collections = ref([]);
    const selectedCollectionId = ref("all");

    const currentUser = ref("");
    const userRole = ref("");
    const userLocale = ref(locale.value);
    const userAvatar = ref(null);
    const userWallpaper = ref(defaultWallpaper());
    const userAccent = ref(loadCachedAccent());
    const userVideoEncodeOrder = ref(setVideoEncodeOrder());

    function applyVideoEncodeOrder(order) {
        userVideoEncodeOrder.value = setVideoEncodeOrder(order);
    }
    const members = ref([]);
    const settings = ref({});
    const aiStatus = ref(null);
    const webdavShares = ref([]);
    const dashboard = ref(null);
    const dashboardLoading = ref(false);
    const dashboardError = ref("");
    const isAdmin = computed(() => userRole.value === "admin");
    const photosPerPage = computed(() => clampPageSize(settings.value.photos_per_page));

    // ── Conflict modal ─────────────────────────────────────────────────────
    const conflictModalVisible = ref(false);
    let conflictResolve = null;

    function askConflictAction() {
        conflictModalVisible.value = true;
        return new Promise((resolve) => {
            conflictResolve = resolve;
        });
    }

    function resolveConflict(action) {
        conflictModalVisible.value = false;
        if (conflictResolve) {
            conflictResolve(action);
            conflictResolve = null;
        }
    }

    function cancelConflict() {
        conflictModalVisible.value = false;
        if (conflictResolve) {
            conflictResolve(null);
            conflictResolve = null;
        }
    }
    // ──────────────────────────────────────────────────────────────────────

    let taskTimer = null;
    const taskListExpanded = ref(false);

    const TASK_POLL_MS_EXPANDED = 500;
    const TASK_POLL_MS_COLLAPSED = 2000;

    const breadcrumbParts = computed(() => {
        if (!currentPath.value) {
            return [];
        }

        const parts = currentPath.value.split("/").filter(Boolean);
        return parts.map((name, index) => ({
            name,
            path: parts.slice(0, index + 1).join("/"),
        }));
    });

    const selectedEntries = computed(() =>
        entries.value.filter((entry) => selectedPaths.value.includes(entry.fullPath))
    );

    const canGoUp = computed(() => Boolean(currentPath.value));
    const canGoBack = computed(() => pathHistoryIndex.value > 0);
    const canGoForward = computed(() => pathHistoryIndex.value < pathHistory.value.length - 1);
    const allSelected = computed(
        () => entries.value.length > 0 && entries.value.every((entry) => isSelected(entry.fullPath))
    );
    const selectAllIndeterminate = computed(
        () => selectedEntries.value.length > 0 && !allSelected.value
    );

    function setMessage(text, kind = "info") {
        message.text = text;
        message.kind = kind;
        message.sequence += 1;
    }

    function isAuthError(error) {
        return error?.status === 401 || error?.status === 403;
    }

    async function apiFetch(url, options = {}) {
        return httpJson(url, options, t("msg.requestFailed"));
    }

    // One place reacts to an expired session: every request goes through the
    // shared client, so a 401 from any component lands here.
    setUnauthorizedHandler(() => {
        if (!loggedIn.value) return;
        loggedIn.value = false;
        stopTaskPolling();
    });

    let dashboardSeq = 0;

    async function refreshDashboard() {
        if (!loggedIn.value) return;
        const seq = ++dashboardSeq;
        dashboardLoading.value = true;
        dashboardError.value = "";
        try {
            const payload = await apiFetch("/api/dashboard", { cache: "no-store" });
            if (seq !== dashboardSeq) return;
            dashboard.value = payload.data || null;
        } catch (error) {
            if (seq !== dashboardSeq) return;
            dashboardError.value = error.message || t("dashboard.loadError");
        } finally {
            if (seq === dashboardSeq) dashboardLoading.value = false;
        }
    }

    const storage = useStorageManager({
        apiFetch,
        isLoggedIn: () => loggedIn.value,
        showMessage: setMessage,
        refreshTasks,
        ensureTaskPolling,
    });

    async function login() {
        if (!loginForm.username || !loginForm.passphrase) {
            setMessage(t("msg.enterCredentials"), "error");
            return;
        }

        busy.login = true;
        try {
            await apiFetch("/api/login", {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                },
                body: JSON.stringify({
                    username: loginForm.username,
                    passphrase: loginForm.passphrase,
                    keep_login: loginForm.keepLogin,
                }),
            });
            loggedIn.value = true;
            currentUser.value = loginForm.username;
            // Fetch role from whoami
            try {
                const whoami = await apiFetch("/api/whoami");
                userRole.value = whoami.data?.role || "user";
                userLocale.value = whoami.data?.locale || locale.value;
                userAvatar.value = whoami.data?.avatar || null;
                userWallpaper.value = normalizeWallpaper(whoami.data?.wallpaper);
                userAccent.value = normalizeAccent(whoami.data?.accent);
                applyVideoEncodeOrder(whoami.data?.video_encode_order);
                setLocale(userLocale.value);
            } catch { userRole.value = "user"; }
            await refreshList();
            await refreshTasks();
            await refreshAlbums();
            await refreshSettings();
            await refreshAiStatus();
            await refreshMembers();
            await refreshWebdavShares();
            await refreshDashboard();
            ensureTaskPolling();
        } catch (error) {
            setMessage(error.message, "error");
        } finally {
            busy.login = false;
        }
    }

    function normalizeEntry(entry) {
        return {
            ...entry,
            fullPath: joinPath(currentPath.value, entry.name),
        };
    }

    async function refreshList() {
        entryOffset.value = 0;
        entries.value = [];
        entryTotal.value = 0;
        selectAllActive.value = false;
        await loadMoreEntries();
    }

    async function loadAllRemainingEntries() {
        if (loadingMore.entries) return false;
        if (entryOffset.value >= entryTotal.value) return true;
        loadingMore.entries = true;
        try {
            const path = encodePath(currentPath.value);
            const url = path ? `/api/list_files/${path}` : "/api/list_files/";
            const params = new URLSearchParams();
            params.set("offset", entryOffset.value.toString());
            params.set("limit", "0"); // unlimited — load all remaining at once
            const payload = await apiFetch(`${url}?${params.toString()}`);
            const data = payload.data || { items: [], total_count: 0 };
            const normalizedItems = (data.items || []).map((entry) => normalizeEntry(entry));
            entries.value.push(...normalizedItems);
            entryTotal.value = data.total_count || entryTotal.value;
            entryOffset.value += normalizedItems.length;
            return true;
        } catch (error) {
            if (isAuthError(error)) {
                loggedIn.value = false;
                stopTaskPolling();
            }
            setMessage(error.message, "error");
            return false;
        } finally {
            loadingMore.entries = false;
        }
    }

    async function loadMoreEntries() {
        if (loadingMore.entries) return false;
        if (entryOffset.value > 0 && entryOffset.value >= entryTotal.value) return false;
        const isInitial = entryOffset.value === 0;
        if (isInitial) busy.list = true;
        loadingMore.entries = true;
        try {
            const path = encodePath(currentPath.value);
            const url = path ? `/api/list_files/${path}` : "/api/list_files/";
            const params = new URLSearchParams();
            params.set("offset", entryOffset.value.toString());
            params.set("limit", photosPerPage.value.toString());
            const fullUrl = `${url}?${params.toString()}`;
            const payload = await apiFetch(fullUrl);
            const data = payload.data || { dirs: [], files: [], items: [], total_count: 0 };

            const normalizedItems = (data.items || []).map((entry) => normalizeEntry(entry));

            if (entryOffset.value === 0) {
                entries.value = normalizedItems;
            } else {
                entries.value.push(...normalizedItems);
            }

            entryTotal.value = data.total_count || 0;
            entryOffset.value += normalizedItems.length;

            if (selectAllActive.value && normalizedItems.length > 0) {
                const newPaths = normalizedItems.map((e) => e.fullPath);
                selectedPaths.value = [...new Set([...selectedPaths.value, ...newPaths])];
            } else {
                selectedPaths.value = selectedPaths.value.filter((pathItem) =>
                    entries.value.some((entry) => entry.fullPath === pathItem)
                );
            }
            return true;
        } catch (error) {
            if (isAuthError(error)) {
                loggedIn.value = false;
                stopTaskPolling();
            }
            setMessage(error.message, "error");
            return false;
        } finally {
            if (isInitial) busy.list = false;
            loadingMore.entries = false;
        }
    }

    function resetPathHistory(path = "") {
        pathHistory.value = [path];
        pathHistoryIndex.value = 0;
    }

    function navigateTo(path, options = {}) {
        const next = normalizeBrowsePath(path);
        const record = options.record !== false;
        if (next === currentPath.value) return;
        if (record) {
            const kept = pathHistory.value.slice(0, pathHistoryIndex.value + 1);
            if (kept[kept.length - 1] !== next) kept.push(next);
            pathHistory.value = kept;
            pathHistoryIndex.value = kept.length - 1;
        }
        currentPath.value = next;
        refreshList();
    }

    function goBack() {
        if (!canGoBack.value) return;
        pathHistoryIndex.value -= 1;
        currentPath.value = pathHistory.value[pathHistoryIndex.value] || "";
        refreshList();
    }

    function goForward() {
        if (!canGoForward.value) return;
        pathHistoryIndex.value += 1;
        currentPath.value = pathHistory.value[pathHistoryIndex.value] || "";
        refreshList();
    }

    function goUp() {
        if (!currentPath.value) {
            return;
        }

        const parts = currentPath.value.split("/").filter(Boolean);
        parts.pop();
        navigateTo(parts.join("/"));
    }

    function openEntry(entry) {
        if (selectedPaths.value.length > 0) {
            toggleSelection({
                path: entry.fullPath,
                additiveKey: true,
                replace: false,
            });
            return;
        }

        if (entry.is_dir) {
            navigateTo(entry.fullPath);
            return;
        }

        downloadEntry(entry);
    }

    function toggleSelection(payload) {
        const value = typeof payload === "string" ? { path: payload } : payload;
        const path = value.path;
        if (!path) {
            return;
        }

        const index = typeof value.index === "number"
            ? value.index
            : entries.value.findIndex((entry) => entry.fullPath === path);

        if (value.shiftKey && lastSelectedIndex.value !== null && index >= 0) {
            const [start, end] = [lastSelectedIndex.value, index].sort((left, right) => left - right);
            const range = entries.value.slice(start, end + 1).map((entry) => entry.fullPath);
            selectedPaths.value = value.additiveKey
                ? [...new Set([...selectedPaths.value, ...range])]
                : range;
            return;
        }

        lastSelectedIndex.value = index >= 0 ? index : lastSelectedIndex.value;
        if (selectedPaths.value.includes(path)) {
            selectedPaths.value = selectedPaths.value.filter((item) => item !== path);
            return;
        }

        selectedPaths.value = value.replace
            ? [path]
            : [...selectedPaths.value, path];
    }

    async function toggleSelectAll() {
        if (allSelected.value && entryOffset.value >= entryTotal.value) {
            selectedPaths.value = [];
            selectAllActive.value = false;
            return;
        }

        selectAllActive.value = true;
        selectedPaths.value = entries.value.map((entry) => entry.fullPath);

        // Load all remaining entries in one shot so operations apply to everything
        if (entryOffset.value < entryTotal.value) {
            await loadAllRemainingEntries();
            selectedPaths.value = entries.value.map((entry) => entry.fullPath);
        }
    }

    function clearSelection() {
        selectedPaths.value = [];
        selectAllActive.value = false;
        lastSelectedIndex.value = null;
    }

    function setSelection(paths) {
        selectedPaths.value = [...new Set(paths)];
        if (selectedPaths.value.length > 0) {
            const lastPath = selectedPaths.value[selectedPaths.value.length - 1];
            lastSelectedIndex.value = entries.value.findIndex((entry) => entry.fullPath === lastPath);
        }
    }

    async function selectAllEntries() {
        selectAllActive.value = true;
        selectedPaths.value = entries.value.map((entry) => entry.fullPath);
        lastSelectedIndex.value = entries.value.length > 0 ? entries.value.length - 1 : null;

        // Load all remaining entries in one shot
        if (entryOffset.value < entryTotal.value) {
            await loadAllRemainingEntries();
            selectedPaths.value = entries.value.map((entry) => entry.fullPath);
            lastSelectedIndex.value = entries.value.length > 0 ? entries.value.length - 1 : null;
        }
    }

    function isSelected(path) {
        return selectedPaths.value.includes(path);
    }

    function downloadEntry(entry) {
        if (entry.is_dir) {
            return;
        }

        // Blob download: a plain navigation/window.open hands the file to
        // Safari's inline viewer on iOS even with an attachment header.
        const path = encodePath(entry.fullPath);
        downloadFileByUrl(`/api/download/${path}`, entry.name).catch((e) => {
            alert(e.message || "Download failed");
        });
    }

    function setViewMode(mode) {
        if (mode === "list" || mode === "grid") {
            viewMode.value = mode;
        }
    }

    async function deletePaths(paths, label) {
        try {
            await apiFetch("/api/delete", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ paths }),
            });
            setMessage(t("msg.deleteTaskSubmitted", { label }));
            await refreshList();
            await refreshTasks();
            ensureTaskPolling();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function deleteEntry(entry) {
        const ok = window.confirm(t("msg.confirmDeleteEntry", { name: entry.name }));
        if (!ok) {
            return;
        }
        await deletePaths([entry.fullPath], entry.name);
    }

    async function deleteSelected() {
        const count = selectedEntries.value.length;
        if (count === 0) {
            return;
        }

        const ok = window.confirm(t("msg.confirmDeleteMultiple", { count }));
        if (!ok) {
            return;
        }

        const paths = selectedEntries.value.map((e) => e.fullPath);
        clearSelection();
        await deletePaths(paths, `${count} ${t("tasks.items")}`);
    }

    async function renameEntry(entry) {
        const nextName = window.prompt(t("msg.enterNewName"), entry.name);
        if (!nextName || nextName === entry.name) {
            return;
        }

        const targetPath = joinPath(currentPath.value, nextName.trim());
        try {
            const path = encodePath(entry.fullPath);
            const query = encodeURIComponent(targetPath);
            await apiFetch(`/api/rename/${path}?new_path=${query}`, { method: "POST" });
            setMessage(t("msg.renamedTo", { name: nextName }));
            await refreshList();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function promptNewFolder() {
        const name = window.prompt(t("msg.newFolderName"));
        if (!name) {
            return;
        }

        const targetPath = joinPath(currentPath.value, name.trim());
        try {
            await apiFetch(`/api/new_folder/${encodePath(targetPath)}`);
            setMessage(t("msg.folderCreated", { name }));
            await refreshList();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function copySelected() {
        if (selectedEntries.value.length === 0) {
            return;
        }

        clipboardMode.value = "copy";
        clipboard.value = selectedEntries.value.map((entry) => ({
            fullPath: entry.fullPath,
            name: entry.name,
        }));
        clearSelection();
        setMessage(t("msg.copiedItems", { count: clipboard.value.length }));
    }

    async function cutSelected() {
        if (selectedEntries.value.length === 0) {
            return;
        }

        clipboardMode.value = "cut";
        clipboard.value = selectedEntries.value.map((entry) => ({
            fullPath: entry.fullPath,
            name: entry.name,
        }));
        clearSelection();
        setMessage(t("msg.cutItems", { count: clipboard.value.length }));
    }

    function cancelClipboard() {
        clipboard.value = [];
        clipboardMode.value = "copy";
        setMessage("");
    }

    async function pasteHere() {
        if (clipboard.value.length === 0) {
            return;
        }

        const conflict = await askConflictAction();
        if (!conflict) {
            return; // user cancelled
        }

        const destination = currentPath.value;
        const items = [...clipboard.value];
        const isCutMode = clipboardMode.value === "cut";
        const endpoint = isCutMode ? "/api/move_files" : "/api/copy_files";
        const actionText = isCutMode ? t("msg.actionMove") : t("msg.actionCopy");

        try {
            await apiFetch(endpoint, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                },
                body: JSON.stringify({
                    sources: items.map((item) => item.fullPath),
                    destinations: items.map((item) =>
                        joinPath(destination, item.name)
                    ),
                    conflict,
                }),
            });
            clipboard.value = [];
            clipboardMode.value = "copy";
            setMessage(t("msg.pasteTaskSubmitted", { action: actionText }));
            await refreshTasks();
            await refreshList();
            ensureTaskPolling();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    function buildRelativePath(file, isDirectoryMode) {
        if (isDirectoryMode && file.webkitRelativePath) {
            return file.webkitRelativePath;
        }

        return file.name;
    }

    async function sendUploadChunk(taskId, uploadPath, fileName, chunk) {
        const formData = new FormData();
        formData.append("task_id", taskId);
        formData.append("path", uploadPath);
        formData.append("file", chunk, fileName);
        await apiFetch("/api/upload_feed", {
            method: "POST",
            body: formData,
        });
    }

    async function uploadFileInChunks(taskId, uploadItem) {
        const { file, relativePath } = uploadItem;
        if (file.size === 0) {
            return;
        }

        const uploadPath = joinPath(currentPath.value, relativePath);
        for (let offset = 0; offset < file.size; offset += UPLOAD_CHUNK_SIZE) {
            if (uploadCancelled.value) return;
            const chunk = file.slice(offset, offset + UPLOAD_CHUNK_SIZE);
            await sendUploadChunk(taskId, uploadPath, file.name, chunk);
        }
    }

    async function uploadFiles(files, isDirectoryMode) {
        const conflict = await askConflictAction();
        if (!conflict) {
            return; // user cancelled
        }

        busy.upload = true;
        uploadCancelled.value = false;
        try {
            const uploadItems = files
                .map((file) => ({
                    file,
                    relativePath: buildRelativePath(file, isDirectoryMode),
                }))
                .sort((left, right) => compareRelativePaths(left.relativePath, right.relativePath));

            const metas = uploadItems.map(({ file, relativePath }) => ({
                bytes: file.size,
                is_dir: false,
                path: relativePath,
            }));

            const createTaskPayload = await apiFetch("/api/upload", {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                },
                body: JSON.stringify({
                    files: metas,
                    destination: currentPath.value,
                    conflict,
                }),
            });

            const taskId = createTaskPayload.data.task_id;
            for (const uploadItem of uploadItems) {
                if (uploadCancelled.value) break;
                await uploadFileInChunks(taskId, uploadItem);
            }

            if (uploadCancelled.value) {
                setMessage(t("msg.uploadCancelled"));
            } else {
                setMessage(t("msg.uploadTaskSubmitted", { count: uploadItems.length }));
            }
            await refreshTasks();
            await refreshList();
        } catch (error) {
            if (!uploadCancelled.value) {
                setMessage(error.message, "error");
            }
        } finally {
            busy.upload = false;
            uploadCancelled.value = false;
        }
    }

    async function refreshTasks() {
        if (!loggedIn.value) {
            return;
        }

        busy.tasks = true;
        try {
            const payload = await apiFetch("/api/get_task_list/");
            const data = payload.data || {};
            tasks.value = Object.entries(data).map(([id, task]) => ({
                id,
                ...task,
            }));
        } catch (error) {
            if (isAuthError(error)) {
                loggedIn.value = false;
                stopTaskPolling();
            }
            // Swallow network errors from background polling — not actionable by the user.
        } finally {
            busy.tasks = false;
        }
    }

    async function clearFinishedTasks() {
        try {
            await apiFetch("/api/clear_finished_tasks", {
                method: "POST",
            });
            await refreshTasks();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function cancelTask(taskId) {
        try {
            await apiFetch(`/api/cancel_task/${taskId}`, {
                method: "POST",
            });
            uploadCancelled.value = true;
            setMessage(t("msg.taskCancelled"));
            await refreshTasks();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function pauseTask(taskId) {
        try {
            await apiFetch(`/api/pause_task/${taskId}`, {
                method: "POST",
            });
            setMessage(t("msg.taskPaused"));
            await refreshTasks();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function resumeTask(taskId) {
        try {
            await apiFetch(`/api/resume_task/${taskId}`, {
                method: "POST",
            });
            setMessage(t("msg.taskResumed"));
            await refreshTasks();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function refreshAlbums() {
        if (!loggedIn.value) {
            return;
        }
        busy.albums = true;
        try {
            const payload = await apiFetch("/api/albums");
            albums.value = payload.data?.albums || [];
            sharedAlbums.value = payload.data?.shared_albums || [];
            const allAlbumNames = [
                ...albums.value.map((a) => a.dir_name),
                ...sharedAlbums.value.map((a) => a.dir_name),
            ];
            if (currentAlbum.value && !allAlbumNames.includes(currentAlbum.value)) {
                currentAlbum.value = "";
                albumPhotos.value = [];
                albumAvailableTags.value = [];
            }
        } catch {
            // Swallow background refresh errors — album operations report their own status.
        } finally {
            busy.albums = false;
        }
    }

    async function createAlbum({ name, description } = {}) {
        if (!name) {
            return false;
        }
        try {
            await apiFetch("/api/albums", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ name: name.trim(), description: description || "" }),
            });
            setMessage(t("msg.albumCreated", { name: name.trim() }));
            await refreshAlbums();
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    async function updateAlbum({ dirName, name, cover, description } = {}) {
        if (!dirName) return false;
        try {
            const body = { dir_name: dirName };
            if (name !== undefined) body.name = name;
            if (cover !== undefined) body.cover = cover;
            if (description !== undefined) body.description = description;
            const payload = await apiFetch("/api/albums/update", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(body),
            });
            const nextDir = payload?.data?.dir_name;
            if (nextDir && currentAlbum.value === dirName && nextDir !== dirName) {
                currentAlbum.value = nextDir;
            }
            setMessage(t("msg.albumUpdated"));
            await refreshAlbums();
            if (nextDir && currentAlbum.value === nextDir) {
                await refreshAlbumPhotos();
            }
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    async function deleteAlbum(dirName) {
        if (!dirName) return false;
        try {
            await apiFetch("/api/albums/delete", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ dir_name: dirName }),
            });
            setMessage(t("msg.albumDeleted"));
            if (currentAlbum.value === dirName) {
                currentAlbum.value = "";
                albumPhotos.value = [];
            }
            await refreshAlbums();
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    async function shareAlbum({ dirName, username, canWrite, canDelete } = {}) {
        if (!dirName || !username) return false;
        try {
            await apiFetch("/api/albums/share", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    dir_name: dirName,
                    username,
                    can_write: !!canWrite,
                    can_delete: !!canDelete,
                }),
            });
            setMessage(t("msg.albumShared", { username }));
            await refreshAlbums();
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    async function unshareAlbum({ dirName, username } = {}) {
        if (!dirName || !username) return false;
        try {
            await apiFetch("/api/albums/unshare", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ dir_name: dirName, username }),
            });
            setMessage(t("msg.albumUnshared", { username }));
            await refreshAlbums();
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    function albumFiltersHaveConstraints({ includeAiText = true } = {}) {
        const { aiText, metadata } = partitionFilterClauses(albumFilters.clauses);
        if (metadata.length > 0) return true;
        if (includeAiText && aiText.length > 0) return true;
        return false;
    }

    function readCollapseBurstsPreference() {
        try {
            return localStorage.getItem("lumeno.collapseBursts") !== "0";
        } catch {
            return true;
        }
    }

    async function setBurstSettings({ algorithm, threshold, embeddingThreshold }) {
        if (!currentAlbum.value) return;
        const nextAlgorithm = algorithm === "embedding" ? "embedding" : "dct";
        const nextThreshold = Math.max(0, Math.min(32, Math.round(Number(threshold) || 0)));
        const nextEmbedding = Math.max(30, Math.min(100, Math.round(Number(embeddingThreshold) || 80)));
        try {
            await apiFetch("/api/albums/update", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    dir_name: currentAlbum.value,
                    burst_algorithm: nextAlgorithm,
                    burst_threshold: nextThreshold,
                    burst_embedding_threshold: nextEmbedding,
                }),
            });
            burstAlgorithm.value = nextAlgorithm;
            burstThreshold.value = nextThreshold;
            burstEmbeddingThreshold.value = nextEmbedding;
            refreshAlbumPhotos();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    function setCollapseBursts(value) {
        collapseBursts.value = !!value;
        burstRefreshAttempts = 0;
        try {
            localStorage.setItem("lumeno.collapseBursts", collapseBursts.value ? "1" : "0");
        } catch {
            /* ignore quota / private mode */
        }
        refreshAlbumPhotos();
    }

    function clearBurstRefreshTimer() {
        if (burstRefreshTimer !== null) {
            window.clearTimeout(burstRefreshTimer);
            burstRefreshTimer = null;
        }
    }

    function scheduleBurstRefresh(data, fetchId) {
        clearBurstRefreshTimer();
        const state = data?.bursts?.state;
        if (state === "ready" || state === "disabled") {
            burstRefreshAttempts = 0;
            return;
        }
        if (state !== "pending" && state !== "stale") return;
        if (++burstRefreshAttempts > 20) return;
        burstRefreshTimer = window.setTimeout(() => {
            if (fetchId === albumPhotosFetchId) refreshAlbumPhotos();
        }, 2500);
    }

    async function fetchAlbumPhotosPage({ offset = 0, limit, filters = null, paths = null } = {}) {
        const body = {
            album: currentAlbum.value,
            filters: filters ?? albumFilters.clauses ?? [],
            offset,
            limit: limit == null ? photosPerPage.value : limit,
        };
        if (collapseBursts.value) body.collapse = "bursts";
        if (Array.isArray(paths) && paths.length > 0) {
            body.paths = paths;
        }
        if (albumSort.value.length > 0) {
            body.sort = albumSort.value.map((s) => `${s.field}:${s.dir}`).join(",");
        }
        const payload = await apiFetch("/api/albums/photos", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body),
        });
        return payload.data || { photos: [], total_count: 0 };
    }

    /** Load full album-index metadata (EXIF, tags, etc.) for specific paths. */
    async function fetchAlbumPhotosByPaths(paths) {
        if (!loggedIn.value || !currentAlbum.value) return [];
        const unique = [...new Set((paths || []).map((p) => String(p || "").trim()).filter(Boolean))];
        if (unique.length === 0) return [];
        const data = await fetchAlbumPhotosPage({ offset: 0, limit: 0, filters: [], paths: unique });
        return data.photos || [];
    }

    /** Paths matching current metadata filters (excludes AI text). null = no restriction. */
    async function fetchFilteredPhotoPaths() {
        if (!loggedIn.value || !currentAlbum.value) return null;
        const { metadata } = partitionFilterClauses(albumFilters.clauses);
        if (metadata.length === 0) return null;

        const data = await fetchAlbumPhotosPage({ offset: 0, limit: 0, filters: metadata });
        const photos = data.photos || [];
        return photos.map((photo) => photo.path).filter(Boolean);
    }

    function mapAiSearchResults(results) {
        return (results || []).map((item) => ({
            ...item,
            name: item.name || String(item.path || "").split("/").pop() || item.path,
            datetime: item.datetime || item.date || "",
            tags: item.tags || [],
            rating: item.rating || 0,
            color_label: item.color_label || [],
        }));
    }

    function restoreAlbumFilters(album) {
        if (!album) {
            albumFilters.clauses = [];
            return;
        }
        const savedFilterItems = loadFilterItemsFromStorage(currentUser.value, album);
        albumFilters.clauses = compileAlbumFilterClauses(savedFilterItems);
    }

    async function refreshAlbumPhotos() {
        const fetchId = ++albumPhotosFetchId;
        clearBurstRefreshTimer();
        albumPhotoOffset.value = 0;
        albumPhotos.value = [];
        albumPhotoTotal.value = 0;
        albumBursts.value = { state: "ready", algorithm: "burst-v1", stack_count: 0 };

        const clauses = [...(albumFilters.clauses || [])];
        const { aiText, metadata } = partitionFilterClauses(clauses);
        const text = String(aiText[0]?.value?.q || "").trim();
        if (text) {
            await applyAiTextFilterToPhotos(
                text,
                Number(aiText[0].value?.minScore) || 0,
                metadata,
                fetchId,
            );
            return;
        }
        await loadMoreAlbumPhotos({ fetchId, filters: clauses });
    }

    async function applyAiTextFilterToPhotos(text, minScore = 0, metadataFilters = null, fetchId = albumPhotosFetchId) {
        if (!loggedIn.value || !currentAlbum.value || !text) return;
        busy.albums = true;
        aiSimilarLoading.value = true;
        try {
            let paths = null;
            try {
                const filters = metadataFilters ?? partitionFilterClauses(albumFilters.clauses).metadata;
                if (filters.length > 0) {
                    const data = await fetchAlbumPhotosPage({ offset: 0, limit: 0, filters });
                    paths = (data.photos || []).map((p) => p.path).filter(Boolean);
                }
            } catch {
                paths = null;
            }
            if (fetchId !== albumPhotosFetchId) return;
            if (Array.isArray(paths) && paths.length === 0) {
                albumPhotos.value = [];
                albumPhotoTotal.value = 0;
                albumPhotoOffset.value = 0;
                return;
            }
            const body = {
                text,
                album: currentAlbum.value,
                scope: "current_album",
                limit: 200,
                min_score: Number(minScore) || 0,
            };
            if (Array.isArray(paths)) body.paths = paths;
            const payload = await apiFetch("/api/ai/search/text", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(body),
            });
            if (fetchId !== albumPhotosFetchId) return;
            const results = mapAiSearchResults(payload.data?.results || []);
            albumPhotos.value = results;
            albumPhotoTotal.value = results.length;
            albumPhotoOffset.value = results.length;
            albumBursts.value = { state: "disabled", algorithm: "burst-v1", stack_count: 0 };
            albumViewMode.value = "photos";
        } catch (error) {
            albumPhotos.value = [];
            albumPhotoTotal.value = 0;
            setMessage(error.message, "error");
        } finally {
            if (fetchId === albumPhotosFetchId) {
                busy.albums = false;
                aiSimilarLoading.value = false;
            }
        }
    }

    async function refreshAlbumDuplicates() {
        if (!loggedIn.value || !currentAlbum.value) {
            albumDuplicateGroups.value = [];
            albumDuplicateTotal.value = 0;
            return;
        }

        busy.albums = true;
        try {
            const params = new URLSearchParams();
            params.set("album", currentAlbum.value);
            params.set("kind", "all");
            params.set("max_distance", "8");
            params.set("offset", "0");
            params.set("limit", "100");
            const payload = await apiFetch(`/api/ai/duplicates?${params.toString()}`);
            const data = payload.data || {};
            albumDuplicateGroups.value = data.groups || [];
            albumDuplicateTotal.value = data.total || 0;
        } catch (error) {
            albumDuplicateGroups.value = [];
            albumDuplicateTotal.value = 0;
            setMessage(error.message, "error");
        } finally {
            busy.albums = false;
        }
    }

    async function showAlbumPhotos() {
        albumViewMode.value = "photos";
        aiSimilarResults.value = [];
        aiSimilarQuery.value = null;
        if (currentAlbum.value && albumPhotos.value.length === 0) {
            await refreshAlbumPhotos();
        }
    }

    async function showAlbumDuplicates() {
        albumViewMode.value = "duplicates";
        aiSimilarResults.value = [];
        aiSimilarQuery.value = null;
        await refreshAlbumDuplicates();
    }

    async function findSimilarPhotos(photo) {
        if (!loggedIn.value || !currentAlbum.value || !photo?.path) {
            return;
        }

        aiSimilarLoading.value = true;
        albumViewMode.value = "similar";
        aiSimilarResults.value = [];
        aiSimilarQuery.value = {
            album: currentAlbum.value,
            path: photo.path,
            name: photo.name || String(photo.path).split("/").pop() || photo.path,
            scope: "current_album",
        };

        try {
            let paths = null;
            try {
                paths = await fetchFilteredPhotoPaths();
            } catch {
                paths = null;
            }
            const body = {
                album: currentAlbum.value,
                path: photo.path,
                scope: "current_album",
                limit: 60,
            };
            if (Array.isArray(paths)) body.paths = paths;
            const payload = await apiFetch("/api/ai/search/similar", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(body),
            });
            const data = payload.data || {};
            aiSimilarResults.value = mapAiSearchResults(data.results || []);
            aiSimilarQuery.value = {
                ...(aiSimilarQuery.value || {}),
                ...(data.query || {}),
                total: data.total ?? aiSimilarResults.value.length,
            };
        } catch (error) {
            aiSimilarResults.value = [];
            setMessage(error.message, "error");
            albumViewMode.value = "photos";
        } finally {
            aiSimilarLoading.value = false;
        }
    }

    async function searchAlbumByText(textOrOptions) {
        const options = typeof textOrOptions === "object" && textOrOptions !== null
            ? textOrOptions
            : { text: textOrOptions };
        const text = String(options.text || "").trim();
        // Album AI tools always search within the current album only.
        const searchScope = "current_album";
        const rawLimit = Number(options.limit);
        const asPage = Boolean(options.page);
        const limit = Number.isFinite(rawLimit) && rawLimit > 0 ? Math.floor(rawLimit) : (asPage ? 10000 : 60);
        const minScore = options.minScore ?? options.min_score;
        const inModal = Boolean(options.inModal);

        if (!loggedIn.value || !currentAlbum.value || !text) {
            if (asPage) mobileSearchPhotos.value = [];
            return inModal ? { results: [], query: { text } } : undefined;
        }

        if (asPage) {
            const fetchId = ++mobileSearchFetchId;
            mobileSearchText.value = text;
            mobileSearchPhotos.value = [];
            mobileSearchLoading.value = true;
            try {
                let paths = null;
                if (Array.isArray(options.filters)) {
                    if (options.filters.length > 0) {
                        const data = await fetchAlbumPhotosPage({ offset: 0, limit: 0, filters: options.filters });
                        paths = (data.photos || []).map((photo) => photo.path).filter(Boolean);
                        if (paths.length === 0) {
                            if (fetchId === mobileSearchFetchId) mobileSearchPhotos.value = [];
                            return;
                        }
                    }
                } else {
                    try {
                        paths = await fetchFilteredPhotoPaths();
                    } catch {
                        paths = null;
                    }
                }
                if (fetchId !== mobileSearchFetchId) return;
                const body = {
                    text,
                    album: currentAlbum.value,
                    scope: searchScope,
                    limit,
                };
                if (minScore !== undefined && minScore !== null && minScore !== "") {
                    body.min_score = Number(minScore);
                }
                if (Array.isArray(paths)) body.paths = paths;
                const payload = await apiFetch("/api/ai/search/text", {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify(body),
                });
                if (fetchId !== mobileSearchFetchId) return;
                if (fetchId !== mobileSearchFetchId) return;
                mobileSearchPhotos.value = mapAiSearchResults(payload.data?.results || []);
            } catch (error) {
                if (fetchId === mobileSearchFetchId) {
                    mobileSearchPhotos.value = [];
                    setMessage(error.message, "error");
                }
            } finally {
                if (fetchId === mobileSearchFetchId) mobileSearchLoading.value = false;
            }
            return;
        }

        if (!inModal) {
            aiSimilarLoading.value = true;
            albumViewMode.value = "text";
            aiSimilarResults.value = [];
            aiSimilarQuery.value = {
                text,
                album: currentAlbum.value,
                scope: searchScope,
                limit,
                min_score: minScore,
            };
        }

        try {
            let paths = null;
            try {
                paths = await fetchFilteredPhotoPaths();
            } catch {
                paths = null;
            }
            const body = {
                text,
                album: currentAlbum.value,
                scope: searchScope,
                limit,
            };
            if (minScore !== undefined && minScore !== null && minScore !== "") {
                body.min_score = Number(minScore);
            }
            if (Array.isArray(paths)) body.paths = paths;
            const payload = await apiFetch("/api/ai/search/text", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(body),
            });
            const data = payload.data || {};
            const results = mapAiSearchResults(data.results || []);
            const query = {
                text,
                album: currentAlbum.value,
                scope: searchScope,
                limit,
                min_score: minScore,
                ...(data.query || {}),
                total: data.total ?? results.length,
            };
            if (inModal) {
                return { results, query, total: query.total };
            }
            aiSimilarResults.value = results;
            aiSimilarQuery.value = query;
        } catch (error) {
            if (inModal) {
                throw error;
            }
            aiSimilarResults.value = [];
            setMessage(error.message, "error");
            albumViewMode.value = "photos";
        } finally {
            if (!inModal) {
                aiSimilarLoading.value = false;
            }
        }
    }

    function clearMobileSearch() {
        mobileSearchFetchId += 1;
        mobileSearchText.value = "";
        mobileSearchPhotos.value = [];
        mobileSearchLoading.value = false;
    }

    /** Load duplicates for the AI tools modal without leaving the photos view. */
    async function loadAlbumDuplicatesForModal() {
        if (!loggedIn.value || !currentAlbum.value) {
            return { groups: [], total: 0 };
        }
        const params = new URLSearchParams();
        params.set("album", currentAlbum.value);
        params.set("kind", "all");
        params.set("max_distance", "8");
        params.set("offset", "0");
        params.set("limit", "100");
        const payload = await apiFetch(`/api/ai/duplicates?${params.toString()}`);
        const data = payload.data || {};
        return {
            groups: data.groups || [],
            total: data.total || 0,
        };
    }

    async function loadMoreAlbumPhotos({ fetchId = albumPhotosFetchId, filters = null } = {}) {
        const clauses = filters ?? albumFilters.clauses;
        const { aiText } = partitionFilterClauses(clauses);
        if (aiText.length > 0) {
            return;
        }
        if (!loggedIn.value || !currentAlbum.value) {
            if (albumPhotoOffset.value === 0) {
                albumPhotos.value = [];
                albumAvailableTags.value = [];
            }
            return;
        }
        if (loadingMore.photos) {
            albumPhotosFillQueued = true;
            return;
        }
        if (albumPhotoOffset.value > 0 && albumPhotoOffset.value >= albumPhotoTotal.value) return;
        const isInitial = albumPhotoOffset.value === 0;
        if (isInitial) busy.albums = true;
        loadingMore.photos = true;
        albumPhotosFillQueued = false;
        try {
            const data = await fetchAlbumPhotosPage({
                offset: albumPhotoOffset.value,
                limit: photosPerPage.value,
                filters: clauses,
            });
            if (fetchId !== albumPhotosFetchId) return;
            const newPhotos = data.photos || [];
            if (albumPhotoOffset.value === 0) {
                albumPhotos.value = newPhotos;
                albumAvailableTags.value = data.available_tags || [];
                albumAvailableCameras.value = data.available_cameras || [];
                albumAvailableLenses.value = data.available_lenses || [];
                albumAvailableFormats.value = data.available_formats || [];
            } else {
                albumPhotos.value.push(...newPhotos);
            }
            albumPhotoTotal.value = data.total_count || 0;
            albumBursts.value = data.bursts || { state: "ready", algorithm: "burst-v1", stack_count: 0 };
            if (data.bursts?.algorithm === "embedding") {
                burstAlgorithm.value = "embedding";
                if (Number.isFinite(Number(data.bursts?.threshold))) {
                    burstEmbeddingThreshold.value = Math.max(30, Math.min(100, Math.round(Number(data.bursts.threshold))));
                }
            } else if (data.bursts) {
                burstAlgorithm.value = "dct";
                if (Number.isFinite(Number(data.bursts?.threshold ?? data.bursts?.max_distance))) {
                    burstThreshold.value = Math.max(0, Math.min(32, Math.round(Number(data.bursts.threshold ?? data.bursts.max_distance))));
                }
            }
            albumPhotoOffset.value += newPhotos.length;
            if (albumPhotoOffset.value === newPhotos.length) scheduleBurstRefresh(data, fetchId);
        } catch { /* ignore transient fetch errors for photo loading */ }
        finally {
            loadingMore.photos = false;
            const current = fetchId === albumPhotosFetchId;
            if (isInitial && current) busy.albums = false;
            if (!albumPhotosFillQueued) return;
            albumPhotosFillQueued = false;
            loadMoreAlbumPhotos({
                fetchId: albumPhotosFetchId,
                filters: current ? clauses : null,
            });
        }
    }

    async function loadAllRemainingAlbumPhotos() {
        const { aiText } = partitionFilterClauses(albumFilters.clauses);
        if (aiText.length > 0) return;
        if (!loggedIn.value || !currentAlbum.value) return;
        if (loadingMore.photos) return;
        if (albumPhotoOffset.value >= albumPhotoTotal.value) return;
        loadingMore.photos = true;
        try {
            const data = await fetchAlbumPhotosPage({
                offset: albumPhotoOffset.value,
                limit: 0,
            });
            const newPhotos = data.photos || [];
            albumPhotos.value.push(...newPhotos);
            albumPhotoTotal.value = data.total_count || albumPhotoTotal.value;
            albumPhotoOffset.value += newPhotos.length;
        } catch { /* ignore transient fetch errors */ }
        finally {
            loadingMore.photos = false;
        }
    }

    async function fetchDateHistogram() {
        if (!loggedIn.value || !currentAlbum.value) {
            dateHistogram.value = [];
            return;
        }
        const albumName = currentAlbum.value;
        try {
            const params = new URLSearchParams();
            params.set("album", albumName);
            const payload = await apiFetch(`/api/albums/date_histogram?${params.toString()}`);
            if (currentAlbum.value !== albumName) return;
            dateHistogram.value = payload.data?.buckets || [];
        } catch {
            if (currentAlbum.value !== albumName) return;
            dateHistogram.value = [];
        }
    }

    async function fetchCollections() {
        if (!loggedIn.value || !currentAlbum.value) {
            collections.value = [];
            return;
        }
        const albumName = currentAlbum.value;
        try {
            const params = new URLSearchParams();
            params.set("album", albumName);
            const payload = await apiFetch(`/api/albums/collections?${params.toString()}`);
            if (currentAlbum.value !== albumName) return;
            collections.value = payload.data?.collections || [];
        } catch {
            if (currentAlbum.value !== albumName) return;
            collections.value = [];
        }
    }

    async function selectCollection(node) {
        selectedCollectionId.value = node.id;
        // Filter compilation and refresh are handled by FilterSidebar
    }

    function albumDirName(input) {
        if (input == null || input === "") return "";
        if (typeof input === "string") return input;
        return String(input.dirName || input.dir_name || input.album || "");
    }

    /// Open one album and start its photo load. Animation timing belongs to the
    /// view that runs it, so nothing here waits for a transition to finish.
    async function selectAlbum(input) {
        const dirName = albumDirName(input);

        currentAlbum.value = dirName;
        const selectedMeta = [...albums.value, ...sharedAlbums.value].find(
            (album) => album?.dir_name === dirName,
        );
        if (selectedMeta) {
            burstAlgorithm.value = selectedMeta.burst_algorithm === "embedding" ? "embedding" : "dct";
            if (Number.isFinite(Number(selectedMeta.burst_threshold))) {
                burstThreshold.value = Math.max(0, Math.min(32, Math.round(Number(selectedMeta.burst_threshold))));
            }
            if (Number.isFinite(Number(selectedMeta.burst_embedding_threshold))) {
                burstEmbeddingThreshold.value = Math.max(30, Math.min(100, Math.round(Number(selectedMeta.burst_embedding_threshold))));
            }
        }
        albumViewMode.value = "photos";
        albumDuplicateGroups.value = [];
        albumDuplicateTotal.value = 0;
        aiSimilarResults.value = [];
        aiSimilarQuery.value = null;
        aiSimilarLoading.value = false;
        selectedCollectionId.value = "all";
        restoreAlbumFilters(dirName);
        clearBurstRefreshTimer();
        burstRefreshAttempts = 0;
        albumPhotos.value = [];
        albumPhotoTotal.value = 0;
        albumPhotoOffset.value = 0;
        albumBursts.value = { state: "ready", algorithm: "burst-v1", stack_count: 0 };

        if (!dirName) {
            albumPhotosFetchId += 1;
            albumPhotosFillQueued = false;
            busy.albums = false;
            dateHistogram.value = [];
            collections.value = [];
            return;
        }

        const openId = ++albumPhotosFetchId;
        albumPhotosFillQueued = false;
        busy.albums = true;
        try {
            // Let FilterSidebar hydrate against this album (no photo request yet).
            await nextTick();
            if (openId !== albumPhotosFetchId || currentAlbum.value !== dirName) return;

            void fetchDateHistogram();
            void fetchCollections();
            void refreshPeopleForAlbum(dirName);
            await refreshAlbumPhotos();
        } finally {
            if (currentAlbum.value === dirName && !loadingMore.photos) {
                busy.albums = false;
            }
        }
    }

    async function importLocalToAlbum(files) {
        if (!currentAlbum.value) {
            setMessage(t("msg.selectAlbumFirst"), "error");
            return;
        }

        const conflict = await askConflictAction();
        if (!conflict) return;

        busy.upload = true;
        uploadCancelled.value = false;
        try {
            const fileList = Array.from(files).filter((f) => f.size > 0);
            if (fileList.length === 0) return;

            const metas = fileList.map((f) => ({ bytes: f.size, name: f.name }));
            const createTaskPayload = await apiFetch("/api/albums/upload", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ album: currentAlbum.value, files: metas, conflict }),
            });
            const taskId = createTaskPayload.data.task_id;
            const stagingPrefix = createTaskPayload.data.staging_prefix;

            for (const file of fileList) {
                if (uploadCancelled.value) break;
                // Feed chunks to the standard upload_feed endpoint.
                // Path is the file's location in the staging dir on the server.
                const uploadPath = `${stagingPrefix}/${file.name}`;
                for (let offset = 0; offset < file.size; offset += UPLOAD_CHUNK_SIZE) {
                    if (uploadCancelled.value) break;
                    const chunk = file.slice(offset, offset + UPLOAD_CHUNK_SIZE);
                    const formData = new FormData();
                    formData.append("task_id", taskId);
                    formData.append("path", uploadPath);
                    formData.append("file", chunk, file.name);
                    await apiFetch("/api/upload_feed", {
                        method: "POST",
                        body: formData,
                    });
                }
            }

            if (!uploadCancelled.value) {
                setMessage(t("msg.importLocalSubmitted", { count: fileList.length }));
                await refreshTasks();
            }
        } catch (error) {
            if (!uploadCancelled.value) {
                setMessage(error.message, "error");
            }
        } finally {
            busy.upload = false;
            uploadCancelled.value = false;
        }
    }

    async function importCloudToAlbum(paths) {
        if (!currentAlbum.value) {
            setMessage(t("msg.selectAlbumFirst"), "error");
            return;
        }
        if (!paths || paths.length === 0) {
            setMessage(t("msg.selectFilesFirst"), "error");
            return;
        }

        const conflict = await askConflictAction();
        if (!conflict) return;

        try {
            await apiFetch("/api/albums/import", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    album: currentAlbum.value,
                    sources: paths,
                    move_files: false,
                    conflict,
                }),
            });
            setMessage(t("msg.importTaskSubmitted", { count: paths.length }));
            await refreshTasks();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function fetchFilesForPicker(path) {
        const encoded = encodePath(path);
        const url = encoded ? `/api/list_files/${encoded}` : "/api/list_files/";
        const payload = await apiFetch(`${url}?limit=0`);
        return payload.data || { items: [], total_count: 0 };
    }



    async function importSelectedToAlbum() {
        if (!currentAlbum.value) {
            setMessage(t("msg.selectAlbumFirst"), "error");
            return;
        }
        if (selectedEntries.value.length === 0) {
            setMessage(t("msg.selectFilesFirst"), "error");
            return;
        }

        const conflict = await askConflictAction();
        if (!conflict) return;

        try {
            const sources = selectedEntries.value.map((item) => item.fullPath);
            await apiFetch("/api/albums/import", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    album: currentAlbum.value,
                    sources,
                    move_files: false,
                    conflict,
                }),
            });
            setMessage(t("msg.importTaskSubmitted", { count: sources.length }));
            await refreshTasks();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function updatePhotoMeta({ path, tags, rating, colorLabel, album }) {
        try {
            const body = { path };
            if (album || currentAlbum.value) body.album = album || currentAlbum.value;
            if (tags !== undefined) body.tags = tags;
            if (rating !== undefined) body.rating = rating;
            if (colorLabel !== undefined) body.color_label = colorLabel;
            const payload = await apiFetch("/api/albums/photo_meta", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(body),
            });
            const data = payload.data;
            if (data) {
                for (const list of [albumPhotos.value, aiSimilarResults.value, mobileSearchPhotos.value]) {
                    const photo = list.find((item) => item.path === path);
                    if (!photo) continue;
                    if (data.tags !== undefined) photo.tags = data.tags;
                    if (data.rating !== undefined) photo.rating = data.rating;
                    if (data.color_label !== undefined) photo.color_label = data.color_label;
                }
            }
            setMessage(t("msg.metadataUpdated"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function rebuildAlbumIndex(album = currentAlbum.value) {
        if (!album) return;
        try {
            const payload = await apiFetch("/api/albums/rebuild_index", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ album }),
            });
            const created = payload.data?.created !== false;
            setMessage(created ? t("msg.rebuildIndexTaskCreated") : t("msg.rebuildIndexTaskAlreadyRunning"));
            await refreshTasks();
            ensureTaskPolling();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function deleteAlbumPhotos(paths) {
        if (!currentAlbum.value || !paths.length) return;
        try {
            const payload = await apiFetch("/api/albums/delete_photos", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ album: currentAlbum.value, paths }),
            });
            setMessage(payload.message || t("msg.deleteCompleted"));
            await refreshAlbumPhotos();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function copyPhotoToAlbum({ photoPath, sourceAlbum, targetAlbum }) {
        try {
            await apiFetch("/api/albums/copy_photo", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    photo_path: photoPath,
                    source_album: sourceAlbum,
                    target_album: targetAlbum,
                }),
            });
            setMessage(t("msg.photoCopiedToAlbum"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    // ── Settings ─────────────────────────────────────────────────────────────

    async function refreshSettings() {
        try {
            const payload = await apiFetch("/api/settings");
            settings.value = payload.data || {};
        } catch {
            settings.value = {};
        }
    }

    async function refreshAiStatus() {
        if (!loggedIn.value) {
            aiStatus.value = null;
            return;
        }

        try {
            const payload = await apiFetch("/api/ai/status");
            aiStatus.value = payload.data || null;
        } catch {
            aiStatus.value = null;
        }
    }

    async function refreshWebdavShares() {
        try {
            const payload = await apiFetch("/api/webdav/shares");
            const data = payload.data || {};
            webdavShares.value = data.shares || [];
        } catch {
            webdavShares.value = [];
        }
    }

    async function updateWebdavShare(update) {
        try {
            const payload = await apiFetch("/api/webdav/shares", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(update),
            });
            webdavShares.value = payload.data?.shares || [];
            if (payload.message) setMessage(payload.message, "success");
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function refreshPreferences() {
        try {
            const payload = await apiFetch("/api/preferences");
            userLocale.value = payload.data?.locale || locale.value;
            userWallpaper.value = normalizeWallpaper(payload.data?.wallpaper);
            userAccent.value = normalizeAccent(payload.data?.accent);
            applyVideoEncodeOrder(payload.data?.video_encode_order);
            setLocale(userLocale.value);
        } catch {
            userLocale.value = locale.value;
        }
    }

    async function refreshMembers() {
        if (!loggedIn.value || userRole.value !== "admin") {
            members.value = [];
            return;
        }
        try {
            const payload = await apiFetch("/api/admin/users");
            members.value = payload.data?.users || [];
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function updateSettings(updates) {
        try {
            const payload = await apiFetch("/api/settings", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ settings: updates }),
            });
            settings.value = payload.data || {};
            await refreshAiStatus();
            setMessage(payload.message || t("msg.settingsSaved"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function scanThumbnailsNow() {
        try {
            const payload = await apiFetch("/api/settings/thumbnail-scan", {
                method: "POST",
            });
            setMessage(payload.message || t("settings.thumbnailScanStarted"));
            await refreshTasks();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function queueAiIndex() {
        try {
            const payload = await apiFetch("/api/ai/index", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    mode: "incremental",
                    features: ["image", "duplicates", "faces"],
                }),
            });
            setMessage(payload.message || t("settings.aiStartIndex"));
            await refreshTasks();
            await refreshAiStatus();
            return payload.data || null;
        } catch (error) {
            setMessage(error.message, "error");
            return null;
        }
    }

    const albumPeople = ref([]);
    const albumPeopleLoading = ref(false);
    const albumFaceSuggestions = ref({ clusters: [], noise_face_ids: [] });
    const albumFaceHints = ref({});
    const albumFaceHintsLoading = ref(false);
    const albumFacesEditable = ref(true);
    const albumFacesOwner = ref("");
    const albumClusterSimilarity = ref(0.62);
    const albumClusterReclustering = ref(false);

    const albumFaceSuggestionsDisplay = computed(() => {
        const base = albumFaceSuggestions.value || {};
        const hints = albumFaceHints.value || {};
        const clusters = (base.clusters || []).map((cluster) => ({
            ...cluster,
            match_suggestions: hints[cluster.cluster_id] || [],
        }));
        return { ...base, clusters };
    });

    async function refreshFaceHintsForAlbum(albumName = currentAlbum.value) {
        if (!loggedIn.value || !albumName) {
            albumFaceHints.value = {};
            return;
        }
        albumFaceHintsLoading.value = true;
        try {
            const payload = await apiFetch(
                `/api/ai/albums/${encodeURIComponent(albumName)}/face-hints`,
            );
            albumFaceHints.value = payload.data?.groups || {};
        } catch {
            albumFaceHints.value = {};
        } finally {
            albumFaceHintsLoading.value = false;
        }
    }

    async function refreshPeopleForAlbum(albumName = currentAlbum.value, opts = {}) {
        const silent = opts.silent === true;
        if (!loggedIn.value || !albumName) {
            albumPeople.value = [];
            albumFaceSuggestions.value = { clusters: [], noise_face_ids: [] };
            albumFaceHints.value = {};
            albumFacesOwner.value = "";
            albumClusterSimilarity.value = 0.62;
            return;
        }
        if (!silent) albumPeopleLoading.value = true;
        try {
            const payload = await apiFetch(`/api/ai/albums/${encodeURIComponent(albumName)}/faces`);
            if (currentAlbum.value !== albumName) return;
            const data = payload.data || {};
            albumPeople.value = data.people || [];
            albumFaceSuggestions.value = data.suggestions || { clusters: [], noise_face_ids: [] };
            albumFacesEditable.value = data.editable !== false;
            albumFacesOwner.value = data.owner || currentUser.value || "";
            const similarity = Number(data.cluster_similarity);
            albumClusterSimilarity.value = Number.isFinite(similarity) ? similarity : 0.62;
        } catch (error) {
            if (currentAlbum.value !== albumName) return;
            albumPeople.value = [];
            albumFaceSuggestions.value = { clusters: [], noise_face_ids: [] };
            albumFaceHints.value = {};
            albumFacesOwner.value = "";
            albumClusterSimilarity.value = 0.62;
            setMessage(error.message, "error");
        } finally {
            if (!silent && currentAlbum.value === albumName) albumPeopleLoading.value = false;
        }
        if (opts.awaitHints) {
            await refreshFaceHintsForAlbum(albumName);
        } else {
            void refreshFaceHintsForAlbum(albumName);
        }
    }

    async function reclusterAlbumFaces(similarity) {
        if (!currentAlbum.value || albumClusterReclustering.value) return;
        const albumName = currentAlbum.value;
        albumClusterReclustering.value = true;
        try {
            const payload = await apiFetch(
                `/api/ai/albums/${encodeURIComponent(albumName)}/faces/recluster`,
                {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify({ similarity }),
                },
            );
            if (currentAlbum.value !== albumName) return;
            const saved = Number(payload.data?.cluster_similarity);
            albumClusterSimilarity.value = Number.isFinite(saved) ? saved : similarity;
            setMessage(payload.message || t("people.reclusterDone"));
            await refreshPeopleForAlbum(albumName, { silent: true, awaitHints: true });
        } catch (error) {
            setMessage(error.message, "error");
        } finally {
            albumClusterReclustering.value = false;
        }
    }

    async function adoptAlbumCluster(body) {
        if (!currentAlbum.value) return;
        albumPeopleLoading.value = true;
        try {
            const payload = await apiFetch(
                `/api/ai/albums/${encodeURIComponent(currentAlbum.value)}/faces/adopt-cluster`,
                {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify(body),
                },
            );
            setMessage(payload.message || t("msg.settingsSaved"));
            await refreshPeopleForAlbum(currentAlbum.value, { silent: true, awaitHints: true });
        } catch (error) {
            setMessage(error.message, "error");
        } finally {
            albumPeopleLoading.value = false;
        }
    }

    async function mergeAlbumCluster(body) {
        if (!currentAlbum.value) return;
        try {
            const payload = await apiFetch(
                `/api/ai/albums/${encodeURIComponent(currentAlbum.value)}/faces/merge-clusters`,
                {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify(body),
                },
            );
            setMessage(payload.message || t("people.mergeDone"));
            await refreshPeopleForAlbum(currentAlbum.value, { silent: true });
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function patchAlbumPerson(personId, updates) {
        if (!currentAlbum.value) return;
        try {
            const payload = await apiFetch(
                `/api/ai/albums/${encodeURIComponent(currentAlbum.value)}/faces/${encodeURIComponent(personId)}`,
                {
                    method: "PATCH",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify(updates),
                },
            );
            setMessage(payload.message || t("msg.settingsSaved"));
            await refreshPeopleForAlbum(currentAlbum.value, { silent: true });
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function removeAlbumPersonCenter(body) {
        if (!currentAlbum.value || !body?.person_id || !body?.center_id) return null;
        try {
            const payload = await apiFetch(
                `/api/ai/albums/${encodeURIComponent(currentAlbum.value)}/faces/remove-center`,
                {
                    method: "POST",
                    headers: { "Content-Type": "application/json" },
                    body: JSON.stringify(body),
                },
            );
            setMessage(payload.message || t("msg.settingsSaved"));
            await refreshPeopleForAlbum(currentAlbum.value, { silent: true, awaitHints: true });
            return payload.data || null;
        } catch (error) {
            setMessage(error.message, "error");
            return null;
        }
    }

    async function deleteAlbumPerson(personId) {
        if (!currentAlbum.value || !personId) return;
        try {
            const payload = await apiFetch(
                `/api/ai/albums/${encodeURIComponent(currentAlbum.value)}/faces/${encodeURIComponent(personId)}`,
                { method: "DELETE" },
            );
            setMessage(payload.message || t("people.deleteDone"));
            const personFilter = (albumFilters.clauses || []).some(
                (clause) =>
                    clause.type === "person"
                    && Array.isArray(clause.value?.personIds)
                    && clause.value.personIds.includes(String(personId)),
            );
            if (personFilter) {
                albumFilters.clauses = (albumFilters.clauses || []).filter(
                    (clause) =>
                        !(
                            clause.type === "person"
                            && Array.isArray(clause.value?.personIds)
                            && clause.value.personIds.includes(String(personId))
                        ),
                );
                await refreshAlbumPhotos();
            }
            await refreshPeopleForAlbum(currentAlbum.value, { silent: true });
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function filterAlbumByPerson(person) {
        const personId = person?.id || person;
        if (!personId || !currentAlbum.value) return;
        albumViewMode.value = "photos";
        aiSimilarResults.value = [];
        aiSimilarQuery.value = null;
        const rest = (albumFilters.clauses || []).filter((c) => c.type !== "person");
        albumFilters.clauses = [
            ...rest,
            {
                type: "person",
                logic: "and",
                enabled: true,
                value: { personIds: [String(personId)] },
            },
        ];
        await refreshAlbumPhotos();
    }

    async function updatePreferences(updates) {
        try {
            const payload = await apiFetch("/api/preferences", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(updates),
            });
            userLocale.value = payload.data?.locale || locale.value;
            userWallpaper.value = normalizeWallpaper(payload.data?.wallpaper);
            userAccent.value = normalizeAccent(payload.data?.accent);
            applyVideoEncodeOrder(payload.data?.video_encode_order);
            setLocale(userLocale.value);
            setMessage(payload.message || t("msg.settingsSaved"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function updatePassword(updates) {
        try {
            const payload = await apiFetch("/api/account/password", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(updates),
            });
            setMessage(payload.message || t("msg.passwordUpdated"));
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    async function uploadAvatar(file) {
        try {
            const formData = new FormData();
            formData.append("avatar", file);
            const payload = await apiFetch("/api/account/avatar", {
                method: "POST",
                body: formData,
            });
            userAvatar.value = payload.data?.avatar || null;
            setMessage(payload.message || t("msg.settingsSaved"));
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    function previewWallpaper(next) {
        const normalized = normalizeWallpaper(next);
        if (JSON.stringify(userWallpaper.value) === JSON.stringify(normalized)) return;
        userWallpaper.value = normalized;
    }

    function previewAccent(next) {
        const normalized = normalizeAccent(next);
        if (userAccent.value === normalized) return;
        userAccent.value = normalized;
    }

    async function uploadWallpaper(file, slot = "source") {
        try {
            const formData = new FormData();
            formData.append("wallpaper", file);
            formData.append("slot", slot === "dark_source" ? "dark_source" : "source");
            const payload = await apiFetch("/api/account/wallpaper", {
                method: "POST",
                body: formData,
            });
            if (payload.data?.wallpaper) {
                userWallpaper.value = normalizeWallpaper(payload.data.wallpaper);
            }
            setMessage(payload.message || t("msg.settingsSaved"));
            return payload.data?.file || null;
        } catch (error) {
            setMessage(error.message, "error");
            return null;
        }
    }

    async function updateMember(updates) {
        try {
            const payload = await apiFetch("/api/admin/users/update", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(updates),
            });
            const updated = payload.data?.user;
            if (updated) {
                members.value = members.value.map((member) =>
                    member.username === updated.username ? updated : member
                );
            }
            setMessage(payload.message || t("msg.settingsSaved"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function deleteMember(username) {
        try {
            const payload = await apiFetch("/api/admin/users/delete", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ username }),
            });
            members.value = members.value.filter((m) => m.username !== username);
            setMessage(payload.message || t("msg.settingsSaved"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function resetMemberPassword(username, newPassword) {
        try {
            const payload = await apiFetch("/api/admin/users/reset_password", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ username, new_password: newPassword }),
            });
            setMessage(payload.message || t("msg.settingsSaved"));
            return true;
        } catch (error) {
            setMessage(error.message, "error");
            return false;
        }
    }

    async function logout() {
        try {
            await apiFetch("/api/logout", { method: "POST" });
        } catch { }
        document.cookie = "auth_token=; Path=/; Max-Age=0";
        loggedIn.value = false;
        currentUser.value = "";
        userRole.value = "";
        userAvatar.value = null;
        userWallpaper.value = defaultWallpaper();
        userAccent.value = defaultAccent();
        applyVideoEncodeOrder();
        userLocale.value = navigator.language.startsWith("zh") ? "zh" : "en";
        setLocale(userLocale.value);
        members.value = [];
        settings.value = {};
        aiStatus.value = null;
        aiSimilarResults.value = [];
        aiSimilarQuery.value = null;
        aiSimilarLoading.value = false;
        albumViewMode.value = "photos";
        dashboard.value = null;
        dashboardError.value = "";
        storage.reset();
        stopTaskPolling();
        entries.value = [];
        tasks.value = [];
        albums.value = [];
        sharedAlbums.value = [];
        albumPhotos.value = [];
        albumAvailableTags.value = [];
        albumAvailableCameras.value = [];
        albumAvailableLenses.value = [];
        currentAlbum.value = "";
        selectedPaths.value = [];
        clipboard.value = [];
        currentPath.value = "";
        resetPathHistory("");
        loginForm.passphrase = "";
        setMessage(t("msg.loggedOut"), "info");
    }

    async function register() {
        if (!loginForm.username || !loginForm.passphrase) {
            setMessage(t("msg.enterCredentials"), "error");
            return;
        }
        busy.login = true;
        try {
            await apiFetch("/api/register", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                    username: loginForm.username,
                    passphrase: loginForm.passphrase,
                }),
            });
            setMessage(t("msg.registerSuccess"), "info");
            await login();
        } catch (error) {
            setMessage(error.message, "error");
        } finally {
            busy.login = false;
        }
    }

    async function resumeSession() {
        try {
            const payload = await apiFetch("/api/whoami");
            currentUser.value = payload.data?.username || "";
            userRole.value = payload.data?.role || "user";
            userLocale.value = payload.data?.locale || locale.value;
            userAvatar.value = payload.data?.avatar || null;
            userWallpaper.value = normalizeWallpaper(payload.data?.wallpaper);
            userAccent.value = normalizeAccent(payload.data?.accent);
            applyVideoEncodeOrder(payload.data?.video_encode_order);
            setLocale(userLocale.value);
            loggedIn.value = true;
        } catch {
            loggedIn.value = false;
            currentUser.value = "";
            userRole.value = "";
            return;
        }

        await refreshList();
        await refreshTasks();
        await refreshAlbums();
        await refreshSettings();
        await refreshAiStatus();
        await refreshPreferences();
        await refreshMembers();
        await refreshWebdavShares();
        await refreshDashboard();
        ensureTaskPolling();
    }

    function ensureTaskPolling() {
        stopTaskPolling();
        const intervalMs = taskListExpanded.value
            ? TASK_POLL_MS_EXPANDED
            : TASK_POLL_MS_COLLAPSED;
        taskTimer = window.setInterval(() => {
            refreshTasks();
        }, intervalMs);
    }

    function setTaskListExpanded(open) {
        const next = Boolean(open);
        if (taskListExpanded.value === next) {
            return;
        }
        taskListExpanded.value = next;
        if (taskTimer !== null) {
            ensureTaskPolling();
            if (next) {
                refreshTasks();
            }
        }
    }

    function stopTaskPolling() {
        if (taskTimer !== null) {
            window.clearInterval(taskTimer);
            taskTimer = null;
        }
    }

    onMounted(() => {
        resumeSession();
    });

    onBeforeUnmount(() => {
        stopTaskPolling();
    });

    return {
        albumAvailableTags,
        albumAvailableCameras,
        albumAvailableLenses,
        albumAvailableFormats,
        albumDuplicateGroups,
        albumDuplicateTotal,
        aiSimilarResults,
        aiSimilarQuery,
        aiSimilarLoading,
        albumFilters,
        albumPhotos,
        mobileSearchPhotos,
        mobileSearchLoading,
        mobileSearchText,
        clearMobileSearch,
        albumPhotoOffset,
        albumPhotoTotal,
        albumBursts,
        collapseBursts,
        burstThreshold,
        burstAlgorithm,
        burstEmbeddingThreshold,
        setCollapseBursts,
        setBurstSettings,
        albumSort,
        albumViewMode,
        albums,
        dateHistogram,
        loadingMore,
        allSelected,
        breadcrumbParts,
        busy,
        cancelClipboard,
        cancelTask,
        pauseTask,
        resumeTask,
        canGoUp,
        canGoBack,
        canGoForward,
        goBack,
        goForward,
        goUp,
        clearFinishedTasks,
        clearSelection,
        clipboard,
        clipboardMode,
        copySelected,
        currentAlbum,
        createAlbum,
        currentPath,
        currentUser,
        dashboard,
        dashboardError,
        dashboardLoading,
        storageReport: storage.report,
        storageLoading: storage.loading,
        storageError: storage.error,
        storageStale: storage.stale,
        storageScanning: storage.scanning,
        storageCleaning: storage.cleaning,
        storageOwner: storage.owner,
        storageMembers: storage.members,
        storageSharedCache: storage.sharedCache,
        storageMembersLoading: storage.membersLoading,
        storageMembersError: storage.membersError,
        loadStorageMembers: storage.loadMembers,
        viewStorageOwner: storage.viewOwner,
        scanAllStorageMembers: storage.scanAll,
        refreshStorageReport: storage.refresh,
        startStorageScan: storage.scan,
        startStorageScanFor: (owner) => storage.scan(owner),
        startStorageCleanup: storage.cleanup,
        userLocale,
        cutSelected,
        deleteAlbum,
        deleteAlbumPhotos,
        deleteEntry,
        deleteMember,
        deleteSelected,
        downloadEntry,
        entries,
        entryOffset,
        entryTotal,
        formatSize,
        formatTime,
        importLocalToAlbum,
        importCloudToAlbum,
        importSelectedToAlbum,
        fetchFilesForPicker,
        fetchAlbumPhotosByPaths,
        isSelected,
        loadAllRemainingAlbumPhotos,
        loadMoreAlbumPhotos,
        loadMoreEntries,
        loggedIn,
        login,
        loginForm,
        logout,
        message,
        members,
        aiStatus,
        albumPeople,
        albumPeopleLoading,
        albumFaceSuggestions,
        albumFaceSuggestionsDisplay,
        albumFaceHintsLoading,
        albumFacesEditable,
        albumFacesOwner,
        albumClusterSimilarity,
        albumClusterReclustering,
        refreshPeopleForAlbum,
        reclusterAlbumFaces,
        refreshFaceHintsForAlbum,
        filterAlbumByPerson,
        adoptAlbumCluster,
        mergeAlbumCluster,
        patchAlbumPerson,
        removeAlbumPersonCenter,
        deleteAlbumPerson,
        navigateTo,
        openEntry,
        pasteHere,
        prettyPath,
        progressPercent,
        promptNewFolder,
        rebuildAlbumIndex,
        refreshAlbumPhotos,
        refreshAlbumDuplicates,
        refreshAlbums,
        refreshAiStatus,
        refreshDashboard,
        refreshList,
        register,
        renameEntry,
        resetMemberPassword,
        selectAlbum,
        selectAllEntries,
        setViewMode,
        selectAllIndeterminate,
        selectedEntries,
        setSelection,
        setMessage,
        shareAlbum,
        showAlbumDuplicates,
        showAlbumPhotos,
        findSimilarPhotos,
        searchAlbumByText,
        loadAlbumDuplicatesForModal,
        sharedAlbums,
        collections,
        selectedCollectionId,
        selectCollection,
        statusLabel,
        statusTone,
        tasks,
        setTaskListExpanded,
        toggleSelectAll,
        toggleSelection,
        unshareAlbum,
        updateAlbum,
        updateMember,
        updatePassword,
        updatePhotoMeta,
        copyPhotoToAlbum,
        updatePreferences,
        updateSettings,
        scanThumbnailsNow,
        queueAiIndex,
        updateWebdavShare,
        uploadAvatar,
        uploadWallpaper,
        previewWallpaper,
        previewAccent,
        uploadFiles,
        userAvatar,
        userWallpaper,
        userAccent,
        userVideoEncodeOrder,
        userRole,
        isAdmin,
        webdavShares,
        settings,
        viewMode,
        conflictModalVisible,
        resolveConflict,
        cancelConflict,
    };
}