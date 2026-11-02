/**
 * The one way a photo path goes into a URL.
 *
 * Segments are encoded individually so `/` stays the separator, and every other
 * byte (including a literal `%`, `#`, `?` or space) is percent-encoded exactly
 * once. Whole-string `encodeURIComponent` and the
 * `encodeURIComponent(...).replace(/%2F/g, "/")` trick produce different URLs
 * for the same file; use this instead.
 */
export function encodePath(path) {
    return String(path ?? "")
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

export function normalizeBrowsePath(input) {
    const text = String(input ?? "").trim().replace(/\\/g, "/");
    const stack = [];
    for (const part of text.split("/")) {
        if (!part || part === ".") continue;
        if (part === "..") stack.pop();
        else stack.push(part);
    }
    return stack.join("/");
}

export function compareRelativePaths(leftPath, rightPath) {
    const leftDepth = leftPath.split("/").filter(Boolean).length;
    const rightDepth = rightPath.split("/").filter(Boolean).length;

    if (leftDepth !== rightDepth) {
        return leftDepth - rightDepth;
    }

    return leftPath < rightPath ? -1 : leftPath > rightPath ? 1 : 0;
}