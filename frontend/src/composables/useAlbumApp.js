import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { formatSize, formatTime, progressPercent, statusLabel, statusTone } from "../utils/format";
import { compareRelativePaths, encodePath, joinPath, prettyPath } from "../utils/path";

const UPLOAD_CHUNK_SIZE = 4 * 1024 * 1024;

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
    });

    const message = reactive({
        kind: "info",
        text: "",
    });

    const currentPath = ref("");
    const entries = ref([]);
    const viewMode = ref("list");
    const tasks = ref([]);
    const selectedPaths = ref([]);
    const loggedIn = ref(false);
    const clipboard = ref([]);
    const lastSelectedIndex = ref(null);

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
    }

    async function apiFetch(url, options = {}) {
        const response = await fetch(url, {
            credentials: "include",
            ...options,
        });

        const contentType = response.headers.get("content-type") || "";
        if (contentType.includes("application/json")) {
            const payload = await response.json();
            if (!payload.success) {
                throw new Error(payload.message || "请求失败");
            }
            return payload;
        }

        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }

        return response;
    }

    async function login() {
        if (!loginForm.username || !loginForm.passphrase) {
            setMessage("请输入用户名和密码。", "error");
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
            setMessage("登录成功，正在加载目录。", "info");
            await refreshList();
            await refreshTasks();
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
        busy.list = true;
        try {
            const path = encodePath(currentPath.value);
            const url = path ? `/api/list_files/${path}` : "/api/list_files/";
            const payload = await apiFetch(url);
            const data = payload.data || { dirs: [], files: [] };
            const normalizedDirs = (data.dirs || []).map((entry) => normalizeEntry(entry));
            const normalizedFiles = (data.files || []).map((entry) => normalizeEntry(entry));
            entries.value = [...normalizedDirs, ...normalizedFiles];
            selectedPaths.value = selectedPaths.value.filter((pathItem) =>
                entries.value.some((entry) => entry.fullPath === pathItem)
            );
            return true;
        } catch (error) {
            if (String(error.message).includes("未验证") || String(error.message).includes("过期")) {
                loggedIn.value = false;
                stopTaskPolling();
            }
            setMessage(error.message, "error");
            return false;
        } finally {
            busy.list = false;
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

    function toggleSelectAll() {
        if (allSelected.value) {
            selectedPaths.value = [];
            return;
        }

        selectedPaths.value = entries.value.map((entry) => entry.fullPath);
    }

    function clearSelection() {
        selectedPaths.value = [];
        lastSelectedIndex.value = null;
    }

    function setSelection(paths) {
        selectedPaths.value = [...new Set(paths)];
        if (selectedPaths.value.length > 0) {
            const lastPath = selectedPaths.value[selectedPaths.value.length - 1];
            lastSelectedIndex.value = entries.value.findIndex((entry) => entry.fullPath === lastPath);
        }
    }

    function selectAllEntries() {
        selectedPaths.value = entries.value.map((entry) => entry.fullPath);
        lastSelectedIndex.value = entries.value.length > 0 ? entries.value.length - 1 : null;
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
            setMessage(`已提交删除任务：${label}`);
            await refreshList();
            await refreshTasks();
            ensureTaskPolling();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function deleteEntry(entry) {
        const ok = window.confirm(`确认删除 ${entry.name} ?`);
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

        const ok = window.confirm(`确认删除选中的 ${count} 项吗？`);
        if (!ok) {
            return;
        }

        const paths = selectedEntries.value.map((e) => e.fullPath);
        clearSelection();
        await deletePaths(paths, `${count} 项`);
    }

    async function renameEntry(entry) {
        const nextName = window.prompt("输入新的名称", entry.name);
        if (!nextName || nextName === entry.name) {
            return;
        }

        const targetPath = joinPath(currentPath.value, nextName.trim());
        try {
            const path = encodePath(entry.fullPath);
            const query = encodeURIComponent(targetPath);
            await apiFetch(`/api/rename/${path}?new_path=${query}`);
            setMessage(`已重命名为 ${nextName}`);
            await refreshList();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function promptNewFolder() {
        const name = window.prompt("新文件夹名称");
        if (!name) {
            return;
        }

        const targetPath = joinPath(currentPath.value, name.trim());
        try {
            await apiFetch(`/api/new_folder/${encodePath(targetPath)}`);
            setMessage(`已创建文件夹 ${name}`);
            await refreshList();
        } catch (error) {
            setMessage(error.message, "error");
        }
    }

    async function copySelected() {
        if (selectedEntries.value.length === 0) {
            return;
        }

        clipboard.value = selectedEntries.value.map((entry) => ({
            fullPath: entry.fullPath,
            name: entry.name,
        }));
        clearSelection();
        setMessage(`已复制 ${clipboard.value.length} 项，请浏览到目标目录后点击粘贴。`);
    }

    function cancelClipboard() {
        clipboard.value = [];
        setMessage("");
    }

    async function pasteHere() {
        if (clipboard.value.length === 0) {
            return;
        }

        const destination = currentPath.value;
        const items = [...clipboard.value];
        clipboard.value = [];

        try {
            await apiFetch("/api/copy_files", {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                },
                body: JSON.stringify({
                    sources: items.map((item) => item.fullPath),
                    destinations: items.map((item) =>
                        joinPath(destination, item.name)
                    ),
                }),
            });
            setMessage("已提交复制任务。");
            await refreshTasks();
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
            const chunk = file.slice(offset, offset + UPLOAD_CHUNK_SIZE);
            await sendUploadChunk(taskId, uploadPath, file.name, chunk);
        }
    }

    async function uploadFiles(files, isDirectoryMode) {
        busy.upload = true;
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
                }),
            });

            const taskId = createTaskPayload.data.task_id;
            for (const uploadItem of uploadItems) {
                await uploadFileInChunks(taskId, uploadItem);
            }

            setMessage(`已提交上传任务，共 ${uploadItems.length} 个文件。`);
            await refreshTasks();
            await refreshList();
        } catch (error) {
            setMessage(error.message, "error");
        } finally {
            busy.upload = false;
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
            if (String(error.message).includes("未验证") || String(error.message).includes("过期")) {
                loggedIn.value = false;
                stopTaskPolling();
            }
            setMessage(error.message, "error");
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

    function logout() {
        document.cookie = "auth_token=; Path=/; Max-Age=0";
        loggedIn.value = false;
        stopTaskPolling();
        entries.value = [];
        tasks.value = [];
        selectedPaths.value = [];
        clipboard.value = [];
        currentPath.value = "";
        loginForm.passphrase = "";
        setMessage("已退出登录。", "info");
    }

    async function resumeSession() {
        loggedIn.value = true;
        const listLoaded = await refreshList();
        if (!listLoaded || !loggedIn.value) {
            loggedIn.value = false;
            if (message.kind === "error") {
                message.text = "前端已加载，请先登录。";
                message.kind = "info";
            }
            return;
        }

        await refreshTasks();
        ensureTaskPolling();
        setMessage("已恢复登录态。", "info");
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
        setMessage("前端已加载，请先登录。", "info");
        resumeSession();
    });

    onBeforeUnmount(() => {
        stopTaskPolling();
    });

    return {
        allSelected,
        breadcrumbParts,
        busy,
        cancelClipboard,
        canGoUp,
        clearFinishedTasks,
        clearSelection,
        clipboard,
        copySelected,
        currentPath,
        deleteEntry,
        deleteSelected,
        entries,
        formatSize,
        formatTime,
        goUp,
        isSelected,
        loggedIn,
        login,
        loginForm,
        logout,
        message,
        navigateTo,
        openEntry,
        pasteHere,
        prettyPath,
        progressPercent,
        promptNewFolder,
        refreshList,
        renameEntry,
        selectAllEntries,
        setViewMode,
        selectAllIndeterminate,
        selectedEntries,
        setSelection,
        setMessage,
        statusLabel,
        statusTone,
        tasks,
        toggleSelectAll,
        toggleSelection,
        uploadFiles,
        viewMode,
        downloadEntry,
    };
}