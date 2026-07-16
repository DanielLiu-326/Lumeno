import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { formatSize, formatTime, progressPercent, statusLabel, statusTone } from "../utils/format";
import { locale, setLocale, t } from "../i18n/index.js";
import { compareRelativePaths, encodePath, joinPath, prettyPath } from "../utils/path";

const UPLOAD_CHUNK_SIZE = 4 * 1024 * 1024;
const DEFAULT_PAGE_SIZE = 100;

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
    const albumPhotos = ref([]);
    const albumPhotoOffset = ref(0);
    const albumPhotoTotal = ref(0);
    const albumAvailableTags = ref([]);
    const albumAvailableCameras = ref([]);
    const albumAvailableLenses = ref([]);
    const albumAvailableFormats = ref([]);
    const albumFilters = reactive({
        from: "",
        to: "",
        tags: "",
        ratings: [],
        colorLabels: [],
        formats: null,  // null = no constraint; string[] = allowed extensions
        cameras: null,  // null = no constraint; string[] = allowed cameras
        lenses: null,   // null = no constraint; string[] = allowed lenses
    });
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
    const members = ref([]);
    const settings = ref({});
    const webdavShares = ref([]);
    const isAdmin = computed(() => userRole.value === "admin");

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
        const headers = {
            "Accept-Language": locale.value,
            ...(options.headers || {}),
        };
        const response = await fetch(url, {
            credentials: "include",
            headers,
            ...options,
        });

        const contentType = response.headers.get("content-type") || "";
        if (contentType.includes("application/json")) {
            const payload = await response.json();
            if (!payload.success) {
                const error = new Error(payload.message || t("msg.requestFailed"));
                error.status = response.status;
                throw error;
            }
            return payload;
        }

        if (!response.ok) {
            const error = new Error(`HTTP ${response.status}`);
            error.status = response.status;
            throw error;
        }

        return response;
    }

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
                setLocale(userLocale.value);
            } catch { userRole.value = "user"; }
            await refreshList();
            await refreshTasks();
            await refreshAlbums();
            await refreshSettings();
            await refreshMembers();
            await refreshWebdavShares();
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
            params.set("limit", DEFAULT_PAGE_SIZE.toString());
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

    function navigateTo(path) {
        currentPath.value = path;
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

        const path = encodePath(entry.fullPath);
        window.open(`/api/download/${path}`, "_blank", "noopener");
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
            await apiFetch("/api/albums/update", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(body),
            });
            setMessage(t("msg.albumUpdated"));
            await refreshAlbums();
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

    async function refreshAlbumPhotos() {
        albumPhotoOffset.value = 0;
        albumPhotos.value = [];
        albumPhotoTotal.value = 0;
        await loadMoreAlbumPhotos();
    }

    async function loadMoreAlbumPhotos() {
        if (!loggedIn.value || !currentAlbum.value) {
            if (albumPhotoOffset.value === 0) {
                albumPhotos.value = [];
                albumAvailableTags.value = [];
            }
            return;
        }
        if (loadingMore.photos) return;
        if (albumPhotoOffset.value > 0 && albumPhotoOffset.value >= albumPhotoTotal.value) return;
        const isInitial = albumPhotoOffset.value === 0;
        if (isInitial) busy.albums = true;
        loadingMore.photos = true;
        try {
            const params = new URLSearchParams();
            params.set("album", currentAlbum.value);
            params.set("offset", albumPhotoOffset.value.toString());
            params.set("limit", DEFAULT_PAGE_SIZE.toString());
            if (albumFilters.from) {
                params.set("from", albumFilters.from);
            }
            if (albumFilters.to) {
                params.set("to", albumFilters.to);
            }
            if (albumFilters.tags) {
                params.set("tags", albumFilters.tags);
            }
            if (albumFilters.ratings.length > 0) {
                params.set("ratings", albumFilters.ratings.join(","));
            }
            if (albumFilters.colorLabels.length > 0) {
                params.set("color_label", albumFilters.colorLabels.join(","));
            }
            if (albumFilters.formats !== null) {
                if (albumFilters.formats.length === 0) {
                    // Contradictory format constraint — no photos can match
                    albumPhotos.value = [];
                    albumPhotoTotal.value = 0;
                    return;
                }
                params.set("formats", albumFilters.formats.join(","));
            }
            if (albumFilters.cameras !== null) {
                if (albumFilters.cameras.length === 0) {
                    albumPhotos.value = [];
                    albumPhotoTotal.value = 0;
                    return;
                }
                params.set("cameras", albumFilters.cameras.join(","));
            }
            if (albumFilters.lenses !== null) {
                if (albumFilters.lenses.length === 0) {
                    albumPhotos.value = [];
                    albumPhotoTotal.value = 0;
                    return;
                }
                params.set("lenses", albumFilters.lenses.join(","));
            }
            if (albumSort.value.length > 0) {
                params.set("sort", albumSort.value.map(s => `${s.field}:${s.dir}`).join(","));
            }
            const payload = await apiFetch(`/api/albums/photos?${params.toString()}`);
            const data = payload.data || { photos: [], available_tags: [], available_cameras: [], available_lenses: [], total_count: 0 };

            const newPhotos = data.photos || [];
            if (albumPhotoOffset.value === 0) {
                albumPhotos.value = newPhotos;
            } else {
                albumPhotos.value.push(...newPhotos);
            }

            if (albumPhotoOffset.value === 0) {
                albumAvailableTags.value = data.available_tags || [];
                albumAvailableCameras.value = data.available_cameras || [];
                albumAvailableLenses.value = data.available_lenses || [];
                albumAvailableFormats.value = data.available_formats || [];
            }
            albumPhotoTotal.value = data.total_count || 0;
            albumPhotoOffset.value += newPhotos.length;
        } catch { /* ignore transient fetch errors for photo loading */ }
        finally {
            if (isInitial) busy.albums = false;
            loadingMore.photos = false;
        }
    }

    async function loadAllRemainingAlbumPhotos() {
        if (!loggedIn.value || !currentAlbum.value) return;
        if (loadingMore.photos) return;
        if (albumPhotoOffset.value >= albumPhotoTotal.value) return;
        loadingMore.photos = true;
        try {
            const params = new URLSearchParams();
            params.set("album", currentAlbum.value);
            params.set("offset", albumPhotoOffset.value.toString());
            params.set("limit", "0"); // unlimited — load all remaining at once
            if (albumFilters.from) params.set("from", albumFilters.from);
            if (albumFilters.to) params.set("to", albumFilters.to);
            if (albumFilters.tags) params.set("tags", albumFilters.tags);
            if (albumFilters.ratings.length > 0) params.set("ratings", albumFilters.ratings.join(","));
            if (albumFilters.colorLabels.length > 0) params.set("color_label", albumFilters.colorLabels.join(","));
            if (albumFilters.formats !== null) {
                if (albumFilters.formats.length === 0) { albumPhotos.value = []; albumPhotoTotal.value = 0; return; }
                params.set("formats", albumFilters.formats.join(","));
            }
            if (albumFilters.cameras !== null) {
                if (albumFilters.cameras.length === 0) { albumPhotos.value = []; albumPhotoTotal.value = 0; return; }
                params.set("cameras", albumFilters.cameras.join(","));
            }
            if (albumFilters.lenses !== null) {
                if (albumFilters.lenses.length === 0) { albumPhotos.value = []; albumPhotoTotal.value = 0; return; }
                params.set("lenses", albumFilters.lenses.join(","));
            }
            if (albumSort.value.length > 0) params.set("sort", albumSort.value.map(s => `${s.field}:${s.dir}`).join(","));
            const payload = await apiFetch(`/api/albums/photos?${params.toString()}`);
            const data = payload.data || { photos: [], total_count: 0 };
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
        try {
            const params = new URLSearchParams();
            params.set("album", currentAlbum.value);
            const payload = await apiFetch(`/api/albums/date_histogram?${params.toString()}`);
            dateHistogram.value = payload.data?.buckets || [];
        } catch {
            dateHistogram.value = [];
        }
    }

    async function fetchCollections() {
        if (!loggedIn.value || !currentAlbum.value) {
            collections.value = [];
            return;
        }
        try {
            const params = new URLSearchParams();
            params.set("album", currentAlbum.value);
            const payload = await apiFetch(`/api/albums/collections?${params.toString()}`);
            collections.value = payload.data?.collections || [];
        } catch {
            collections.value = [];
        }
    }

    async function selectCollection(node) {
        selectedCollectionId.value = node.id;
        // Filter compilation and refresh are handled by FilterSidebar
    }

    async function selectAlbum(dirName) {
        currentAlbum.value = dirName;
        selectedCollectionId.value = "all";
        // Reset filters when switching albums
        albumFilters.from = "";
        albumFilters.to = "";
        albumFilters.tags = "";
        albumFilters.ratings = [];
        albumFilters.colorLabels = [];
        albumFilters.formats = null;
        albumFilters.cameras = null;
        albumFilters.lenses = null;
        // Wait for FilterSidebar's watcher to compile saved filters into albumFilters
        // before triggering the photo refresh.
        await nextTick();
        await Promise.all([refreshAlbumPhotos(), fetchDateHistogram(), fetchCollections()]);
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
        const encoded = path ? encodeURIComponent(path).replace(/%2F/g, "/") : "";
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

    async function updatePhotoMeta({ path, tags, rating, colorLabel }) {
        try {
            const body = { path };
            if (currentAlbum.value) body.album = currentAlbum.value;
            if (tags !== undefined) body.tags = tags;
            if (rating !== undefined) body.rating = rating;
            if (colorLabel !== undefined) body.color_label = colorLabel;
            const payload = await apiFetch("/api/albums/photo_meta", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(body),
            });
            // Update the photo in-place without full reload
            const photo = albumPhotos.value.find((p) => p.path === path);
            if (photo && payload.data) {
                if (payload.data.tags !== undefined) photo.tags = payload.data.tags;
                if (payload.data.rating !== undefined) photo.rating = payload.data.rating;
                if (payload.data.color_label !== undefined) photo.color_label = payload.data.color_label;
            }
            setMessage(t("msg.metadataUpdated"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function rebuildAlbumIndex() {
        if (!currentAlbum.value) return;
        try {
            await apiFetch("/api/albums/rebuild_index", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ album: currentAlbum.value }),
            });
            setMessage(t("msg.rebuildIndexTaskCreated"));
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
            setMessage(payload.message || t("msg.settingsSaved"));
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function updatePreferences(updates) {
        try {
            const payload = await apiFetch("/api/preferences", {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify(updates),
            });
            userLocale.value = payload.data?.locale || locale.value;
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
        userLocale.value = navigator.language.startsWith("zh") ? "zh" : "en";
        setLocale(userLocale.value);
        members.value = [];
        settings.value = {};
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
        await refreshPreferences();
        await refreshMembers();
        await refreshWebdavShares();
        ensureTaskPolling();
        setMessage(t("msg.sessionRestored"), "info");
    }

    function ensureTaskPolling() {
        stopTaskPolling();
        taskTimer = window.setInterval(() => {
            refreshTasks();
        }, 2500);
    }

    function stopTaskPolling() {
        if (taskTimer !== null) {
            window.clearInterval(taskTimer);
            taskTimer = null;
        }
    }

    onMounted(() => {
        setMessage(t("msg.pleaseLogin"), "info");
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
        albumFilters,
        albumPhotos,
        albumPhotoOffset,
        albumPhotoTotal,
        albumSort,
        albums,
        dateHistogram,
        loadingMore,
        allSelected,
        breadcrumbParts,
        busy,
        cancelClipboard,
        cancelTask,
        canGoUp,
        clearFinishedTasks,
        clearSelection,
        clipboard,
        clipboardMode,
        copySelected,
        currentAlbum,
        createAlbum,
        currentPath,
        currentUser,
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
        goUp,
        importLocalToAlbum,
        importCloudToAlbum,
        importSelectedToAlbum,
        fetchFilesForPicker,
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
        navigateTo,
        openEntry,
        pasteHere,
        prettyPath,
        progressPercent,
        promptNewFolder,
        rebuildAlbumIndex,
        refreshAlbumPhotos,
        refreshAlbums,
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
        sharedAlbums,
        collections,
        selectedCollectionId,
        selectCollection,
        statusLabel,
        statusTone,
        tasks,
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
        updateWebdavShare,
        uploadAvatar,
        uploadFiles,
        userAvatar,
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