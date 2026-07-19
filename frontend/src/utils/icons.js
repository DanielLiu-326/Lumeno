import { h, defineComponent } from "vue";

function icon(paths, viewBox = "0 0 20 20", fill = "currentColor") {
  return defineComponent({
    inheritAttrs: false,
    render() {
      return h(
        "svg",
        { xmlns: "http://www.w3.org/2000/svg", viewBox, fill, ...this.$attrs },
        paths.map((p) =>
          typeof p === "string"
            ? h("path", { d: p })
            : h("path", p),
        ),
      );
    },
  });
}

function iconStroke(d, viewBox = "0 0 24 24") {
  return defineComponent({
    inheritAttrs: false,
    render() {
      return h(
        "svg",
        {
          xmlns: "http://www.w3.org/2000/svg",
          viewBox,
          fill: "none",
          stroke: "currentColor",
          "stroke-width": "1.5",
          ...this.$attrs,
        },
        [h("path", { "stroke-linecap": "round", "stroke-linejoin": "round", d })],
      );
    },
  });
}

// ── Navigation ──────────────────────────────────────────
export const IconArrowLeft = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M9.707 14.707a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414l4-4a1 1 0 011.414 1.414L7.414 9H15a1 1 0 110 2H7.414l2.293 2.293a1 1 0 010 1.414z" },
]);

export const IconChevronLeft = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M12.707 5.293a1 1 0 010 1.414L9.414 10l3.293 3.293a1 1 0 01-1.414 1.414l-4-4a1 1 0 010-1.414l4-4a1 1 0 011.414 0z" },
]);

export const IconChevronRight = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z" },
]);

export const IconExpand = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M3 4a1 1 0 011-1h4a1 1 0 010 2H6.414l2.293 2.293a1 1 0 01-1.414 1.414L5 6.414V8a1 1 0 01-2 0V4zm9 1a1 1 0 010-2h4a1 1 0 011 1v4a1 1 0 01-2 0V6.414l-2.293 2.293a1 1 0 11-1.414-1.414L13.586 5H12zm-8 7a1 1 0 012 0v1.586l2.293-2.293a1 1 0 111.414 1.414L6.414 15H8a1 1 0 010 2H4a1 1 0 01-1-1v-4zm13 0a1 1 0 10-2 0v1.586l-2.293-2.293a1 1 0 00-1.414 1.414L13.586 15H12a1 1 0 100 2h4a1 1 0 001-1v-4z" },
]);

export const IconHome = icon([
  "M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h2a1 1 0 001-1v-2a1 1 0 011-1h2a1 1 0 011 1v2a1 1 0 001 1h2a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z",
]);

// Calendar icon for "On This Day"
export const IconCalendarDay = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M6 2a1 1 0 00-1 1v1H4a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V6a2 2 0 00-2-2h-1V3a1 1 0 10-2 0v1H7V3a1 1 0 00-1-1zm0 5a1 1 0 000 2h8a1 1 0 100-2H6z" },
]);

// Sparkles icon for "Daily Picks"
export const IconSparkles = icon([
  "M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z",
]);

// Classic magnifying glass (ring + handle, no plus in the center)
export const IconSearch = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z" },
]);

// Magic wand for AI tools
export const IconWand = defineComponent({
  inheritAttrs: false,
  render() {
    return h(
      "svg",
      {
        xmlns: "http://www.w3.org/2000/svg",
        viewBox: "0 0 24 24",
        fill: "none",
        stroke: "currentColor",
        "stroke-width": "1.5",
        "stroke-linecap": "round",
        "stroke-linejoin": "round",
        ...this.$attrs,
      },
      [
        h("path", { d: "M15 4V2" }),
        h("path", { d: "M15 16v-2" }),
        h("path", { d: "M8 9h2" }),
        h("path", { d: "M20 9h2" }),
        h("path", { d: "m17.8 11.8 1.2 1.2" }),
        h("path", { d: "m17.8 6.2 1.2-1.2" }),
        h("path", { d: "m3 21 9-9" }),
        h("path", { d: "M12.2 11.8 6.2 17.8" }),
      ],
    );
  },
});

export const IconLogout = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M3 3a1 1 0 00-1 1v12a1 1 0 102 0V4a1 1 0 00-1-1zm10.293 9.293a1 1 0 001.414 1.414l3-3a1 1 0 000-1.414l-3-3a1 1 0 10-1.414 1.414L14.586 9H7a1 1 0 100 2h7.586l-1.293 1.293z" },
]);

// ── Actions ─────────────────────────────────────────────
export const IconRefresh = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M4 2a1 1 0 011 1v2.101a7.002 7.002 0 0111.601 2.566 1 1 0 11-1.885.666A5.002 5.002 0 005.999 7H9a1 1 0 010 2H4a1 1 0 01-1-1V3a1 1 0 011-1zm.008 9.057a1 1 0 011.276.61A5.002 5.002 0 0014.001 13H11a1 1 0 110-2h5a1 1 0 011 1v5a1 1 0 11-2 0v-2.101a7.002 7.002 0 01-11.601-2.566 1 1 0 01.61-1.276z" },
]);

export const IconList = icon([
  "M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm1 4a1 1 0 100 2h12a1 1 0 100-2H4z",
]);

export const IconGrid = icon([
  "M3 3h6v6H3V3zm8 0h6v6h-6V3zM3 11h6v6H3v-6zm8 0h6v6h-6v-6z",
]);

export const IconBell = icon([
  "M10 2a5 5 0 00-5 5v2.17c0 .53-.21 1.04-.59 1.42L3.29 11.7A1 1 0 004 13h12a1 1 0 00.71-1.71l-1.12-1.12A2 2 0 0115 8.76V7a5 5 0 00-5-5zm0 16a2.5 2.5 0 002.45-2h-4.9A2.5 2.5 0 0010 18z",
]);

export const IconTasks = icon([
  "M4 3a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V5a2 2 0 00-2-2H4zm1.5 3a1 1 0 100 2h4a1 1 0 100-2h-4zm0 4a1 1 0 100 2h4a1 1 0 100-2h-4zm7.6-1.7a1 1 0 10-1.4 1.4l.6.6-.6.6a1 1 0 101.4 1.4l.6-.6.6.6a1 1 0 001.4-1.4l-.6-.6.6-.6a1 1 0 00-1.4-1.4l-.6.6-.6-.6z",
]);

export const IconMoon = icon([
  "M14.53 12.47A6 6 0 017.53 5.47 7 7 0 1014.53 12.47z",
]);

export const IconSun = icon([
  "M10 4a1 1 0 011-1h0a1 1 0 010 2h0a1 1 0 01-1-1zm0 12a1 1 0 011-1h0a1 1 0 010 2h0a1 1 0 01-1-1zM5.05 6.46a1 1 0 011.41 0l.01.01a1 1 0 11-1.42 1.41l-.01-.01a1 1 0 010-1.41zm8.48 8.48a1 1 0 011.41 0l.01.01a1 1 0 11-1.42 1.41l-.01-.01a1 1 0 010-1.41zM4 10a1 1 0 011-1h0a1 1 0 010 2h0a1 1 0 01-1-1zm12 0a1 1 0 011-1h0a1 1 0 010 2h0a1 1 0 01-1-1zM5.06 15.94a1 1 0 011.41-1.41l.01.01a1 1 0 11-1.42 1.41l-.01-.01zm8.48-8.48a1 1 0 011.41-1.41l.01.01a1 1 0 11-1.42 1.41l-.01-.01zM10 7a3 3 0 100 6 3 3 0 000-6z",
]);

export const IconPower = icon([
  "M11 2a1 1 0 10-2 0v6a1 1 0 102 0V2z",
  "M6.28 3.78a1 1 0 10-1.56-1.25A7 7 0 1015.28 2.5a1 1 0 10-1.56 1.25A5 5 0 116.28 3.78z",
]);

export const IconMenu = icon([
  "M3 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm0 5a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm1 4a1 1 0 100 2h12a1 1 0 100-2H4z",
]);

export const IconUpload = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M3 17a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zM6.293 6.707a1 1 0 010-1.414l3-3a1 1 0 011.414 0l3 3a1 1 0 01-1.414 1.414L11 5.414V13a1 1 0 11-2 0V5.414L7.707 6.707a1 1 0 01-1.414 0z" },
]);

export const IconDownload = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M3 17a1 1 0 011-1h12a1 1 0 110 2H4a1 1 0 01-1-1zm3.293-7.707a1 1 0 011.414 0L9 10.586V3a1 1 0 112 0v7.586l1.293-1.293a1 1 0 111.414 1.414l-3 3a1 1 0 01-1.414 0l-3-3a1 1 0 010-1.414z" },
]);

export const IconTrash = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M9 2a1 1 0 00-.894.553L7.382 4H4a1 1 0 000 2v10a2 2 0 002 2h8a2 2 0 002-2V6a1 1 0 100-2h-3.382l-.724-1.447A1 1 0 0011 2H9zM7 8a1 1 0 012 0v6a1 1 0 11-2 0V8zm5-1a1 1 0 00-1 1v6a1 1 0 102 0V8a1 1 0 00-1-1z" },
]);

export const IconPencil = icon([
  "M13.586 3.586a2 2 0 112.828 2.828l-.793.793-2.828-2.828.793-.793zM11.379 5.793L3 14.172V17h2.828l8.38-8.379-2.83-2.828z",
]);

export const IconClose = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z" },
]);

export const IconCheck = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" },
]);

// ── Files / Folders ─────────────────────────────────────
export const IconFolder = icon([
  "M2 6a2 2 0 012-2h5l2 2h5a2 2 0 012 2v6a2 2 0 01-2 2H4a2 2 0 01-2-2V6z",
]);

export const IconFolderAdd = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M4 4a2 2 0 00-2 2v8a2 2 0 002 2h12a2 2 0 002-2V8a2 2 0 00-2-2h-5L9 4H4zm5 5a1 1 0 10-2 0v1H6a1 1 0 100 2h1v1a1 1 0 102 0v-1h1a1 1 0 100-2H9V9z" },
]);

export const IconFile = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M4 4a2 2 0 012-2h4.586A2 2 0 0112 2.586L15.414 6A2 2 0 0116 7.414V16a2 2 0 01-2 2H6a2 2 0 01-2-2V4z" },
]);

export const IconImage = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M4 3a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V5a2 2 0 00-2-2H4zm12 12H4l4-8 3 6 2-4 3 6z" },
]);

export const IconPlus = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M10 5a1 1 0 011 1v3h3a1 1 0 110 2h-3v3a1 1 0 11-2 0v-3H6a1 1 0 110-2h3V6a1 1 0 011-1z" },
]);

// ── Clipboard ───────────────────────────────────────────
export const IconClipboardCopy = icon([
  "M8 2a1 1 0 000 2h2a1 1 0 100-2H8z",
  "M3 5a2 2 0 012-2 3 3 0 003 3h2a3 3 0 003-3 2 2 0 012 2v6h-4.586l1.293-1.293a1 1 0 00-1.414-1.414l-3 3a1 1 0 000 1.414l3 3a1 1 0 001.414-1.414L10.414 13H15v3a2 2 0 01-2 2H5a2 2 0 01-2-2V5zM15 11h2a1 1 0 110 2h-2v-2z",
]);

export const IconClipboardPaste = icon([
  "M8 3a1 1 0 011-1h2a1 1 0 110 2H9a1 1 0 01-1-1z",
  "M6 3a2 2 0 00-2 2v11a2 2 0 002 2h8a2 2 0 002-2V5a2 2 0 00-2-2 3 3 0 01-3 3H9a3 3 0 01-3-3z",
]);

export const IconScissors = iconStroke(
  "M7 6a2 2 0 11-4 0 2 2 0 014 0zm0 12a2 2 0 11-4 0 2 2 0 014 0zm0-11l10 10M7 17L17 7",
);

// ── Misc ────────────────────────────────────────────────
export const IconCloudUpload = iconStroke(
  "M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12",
);

export const IconFolderOutline = iconStroke(
  "M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z",
);

export const IconAlbum = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M4 3a2 2 0 00-2 2v10a2 2 0 002 2h12a2 2 0 002-2V5a2 2 0 00-2-2H4zm12 12H4l4-8 3 6 2-4 3 6z" },
  "M6 17h12a2 2 0 002-2V7a1 1 0 00-1 1v7a2 2 0 01-2 2H6z",
]);

export const IconGear = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M11.49 3.17c.38-1.56 2.6-1.56 2.98 0a1.532 1.532 0 002.286.948c1.372-.836 2.942.734 2.106 2.106a1.532 1.532 0 00.947 2.287c1.561.379 1.561 2.6 0 2.978a1.532 1.532 0 00-.947 2.287c.836 1.372-.734 2.942-2.106 2.106a1.532 1.532 0 00-2.287.947c-.378 1.561-2.598 1.561-2.978 0a1.533 1.533 0 00-2.287-.947c-1.372.836-2.942-.734-2.106-2.106a1.533 1.533 0 00-.947-2.287c-1.561-.379-1.561-2.6 0-2.978a1.532 1.532 0 00.947-2.287c-.836-1.372.734-2.942 2.106-2.106A1.532 1.532 0 0011.49 3.17zM13 10a3 3 0 11-6 0 3 3 0 016 0z" },
], "0 0 20 20");

export const IconStar = icon([
  "M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z",
]);

export const IconFilter = icon([
  "M3 3a1 1 0 011-1h12a1 1 0 011 1v3a1 1 0 01-.293.707L12 11.414V15a1 1 0 01-.293.707l-2 2A1 1 0 018 17v-5.586L3.293 6.707A1 1 0 013 6V3z",
]);

export const IconDatabase = icon([
  { "fill-rule": "evenodd", "clip-rule": "evenodd", d: "M3 5a2 2 0 012-2h10a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2V5zm11 1H6v3h8V6zM6 11v3h3v-3H6z" },
]);

export const IconCamera = iconStroke(
  "M6.827 6.175A2.31 2.31 0 0 1 5.186 7.23c-.38.054-.757.112-1.134.175C2.999 7.58 2.25 8.507 2.25 9.574V18a2.25 2.25 0 0 0 2.25 2.25h15A2.25 2.25 0 0 0 21.75 18V9.574c0-1.067-.75-1.994-1.802-2.169a47.865 47.865 0 0 0-1.134-.175 2.31 2.31 0 0 1-1.64-1.055l-.822-1.316a2.192 2.192 0 0 0-1.736-1.039 48.774 48.774 0 0 0-5.232 0 2.192 2.192 0 0 0-1.736 1.039l-.821 1.316Z M16.5 12.75a4.5 4.5 0 1 1-9 0 4.5 4.5 0 0 1 9 0Z",
);

export const IconSort = icon([
  "M3 3a1 1 0 000 2h11a1 1 0 100-2H3zm0 4a1 1 0 000 2h7a1 1 0 100-2H3zm0 4a1 1 0 100 2h4a1 1 0 100-2H3zm12-3a1 1 0 10-2 0v5.586l-1.293-1.293a1 1 0 00-1.414 1.414l3 3a1 1 0 001.414 0l3-3a1 1 0 00-1.414-1.414L15 13.586V8z",
]);

// Import: arrow pointing right into a photo album frame
export const IconImport = defineComponent({
  inheritAttrs: false,
  render() {
    return h(
      "svg",
      {
        xmlns: "http://www.w3.org/2000/svg",
        viewBox: "0 0 24 24",
        fill: "none",
        stroke: "currentColor",
        "stroke-width": "1.5",
        "stroke-linecap": "round",
        "stroke-linejoin": "round",
        ...this.$attrs,
      },
      [
        // Arrow pointing right into the album
        h("path", { d: "M2.25 12H12M9 9l3 3-3 3" }),
        // Album/photo frame (rounded rectangle on right half)
        h("path", { d: "M13.5 3.75h6.75a1.5 1.5 0 0 1 1.5 1.5v13.5a1.5 1.5 0 0 1-1.5 1.5H13.5a1.5 1.5 0 0 1-1.5-1.5V5.25a1.5 1.5 0 0 1 1.5-1.5z" }),
        // Landscape inside the photo frame (mountain silhouette)
        h("path", { d: "M14.25 15.75l1.875-3 1.875 2.25 1.5-2.25 1.5 3" }),
      ],
    );
  },
});

export const IconZoomIn = defineComponent({
  inheritAttrs: false,
  render() {
    return h(
      "svg",
      { xmlns: "http://www.w3.org/2000/svg", viewBox: "0 0 20 20", fill: "currentColor", ...this.$attrs },
      [
        h("path", { d: "M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z" }),
        h("path", { d: "M8 5a.75.75 0 01.75.75v1.5h1.5a.75.75 0 010 1.5h-1.5v1.5a.75.75 0 01-1.5 0v-1.5h-1.5a.75.75 0 010-1.5h1.5v-1.5A.75.75 0 018 5z" }),
      ],
    );
  },
});

export const IconZoomOut = defineComponent({
  inheritAttrs: false,
  render() {
    return h(
      "svg",
      { xmlns: "http://www.w3.org/2000/svg", viewBox: "0 0 20 20", fill: "currentColor", ...this.$attrs },
      [
        h("path", { d: "M8 4a4 4 0 100 8 4 4 0 000-8zM2 8a6 6 0 1110.89 3.476l4.817 4.817a1 1 0 01-1.414 1.414l-4.816-4.816A6 6 0 012 8z" }),
        h("path", { d: "M5.25 7.25a.75.75 0 000 1.5h5.5a.75.75 0 000-1.5h-5.5z" }),
      ],
    );
  },
});

// Star with a horizontal line through it (for "no rating" / 0-star)
export const IconStarOff = defineComponent({
  inheritAttrs: false,
  render() {
    return h(
      "svg",
      { xmlns: "http://www.w3.org/2000/svg", viewBox: "0 0 20 20", fill: "currentColor", ...this.$attrs },
      [
        h("path", { d: "M9.049 2.927c.3-.921 1.603-.921 1.902 0l1.07 3.292a1 1 0 00.95.69h3.462c.969 0 1.371 1.24.588 1.81l-2.8 2.034a1 1 0 00-.364 1.118l1.07 3.292c.3.921-.755 1.688-1.54 1.118l-2.8-2.034a1 1 0 00-1.175 0l-2.8 2.034c-.784.57-1.838-.197-1.539-1.118l1.07-3.292a1 1 0 00-.364-1.118L2.98 8.72c-.783-.57-.38-1.81.588-1.81h3.461a1 1 0 00.951-.69l1.07-3.292z", opacity: "0.3" }),
        h("line", { x1: "3", y1: "10", x2: "17", y2: "10", stroke: "currentColor", "stroke-width": "2.5", "stroke-linecap": "round" }),
      ],
    );
  },
});
