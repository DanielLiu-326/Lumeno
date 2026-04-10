export function formatSize(bytes) {
    if (bytes === null || bytes === undefined) {
        return "-";
    }

    const numeric = Number(bytes);
    if (!Number.isFinite(numeric)) {
        return "-";
    }

    if (numeric < 1024) {
        return `${numeric} B`;
    }

    const units = ["KB", "MB", "GB", "TB"];
    let value = numeric / 1024;
    let index = 0;

    while (value >= 1024 && index < units.length - 1) {
        value /= 1024;
        index += 1;
    }

    return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[index]}`;
}

export function formatTime(timestamp) {
    if (!timestamp) {
        return "-";
    }

    return new Date(Number(timestamp) * 1000).toLocaleString("zh-CN", {
        hour12: false,
    });
}

export function progressPercent(task) {
    if (task.total_cost > 0) {
        return Math.min(100, Math.round((task.processed_cost / task.total_cost) * 100));
    }

    if (task.total_count > 0) {
        return Math.min(100, Math.round((task.processed_count / task.total_count) * 100));
    }

    return task.status === 3 ? 100 : 0;
}

export function statusLabel(status) {
    return {
        0: "等待中",
        1: "执行中",
        2: "已暂停",
        3: "已完成",
        4: "已取消",
        5: "失败",
    }[status] || "未知";
}

export function statusTone(status) {
    return {
        0: "bg-stone-200 text-stone-700",
        1: "bg-amber-100 text-amber-800",
        2: "bg-slate-200 text-slate-700",
        3: "bg-emerald-100 text-emerald-800",
        4: "bg-rose-100 text-rose-700",
        5: "bg-red-100 text-red-700",
    }[status] || "bg-stone-200 text-stone-700";
}