export function encodePath(path) {
    return path
        .split("/")
        .filter((segment) => segment.length > 0)
        .map((segment) => encodeURIComponent(segment))
        .join("/");
}

export function joinPath(base, child) {
    return [base, child].filter(Boolean).join("/");
}

export function prettyPath(path) {
    return path ? `/${path}` : "/";
}

export function compareRelativePaths(leftPath, rightPath) {
    const leftDepth = leftPath.split("/").filter(Boolean).length;
    const rightDepth = rightPath.split("/").filter(Boolean).length;

    if (leftDepth !== rightDepth) {
        return leftDepth - rightDepth;
    }

    return leftPath < rightPath ? -1 : leftPath > rightPath ? 1 : 0;
}