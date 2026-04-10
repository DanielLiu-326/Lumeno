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

export const IconHome = icon([
  "M10.707 2.293a1 1 0 00-1.414 0l-7 7a1 1 0 001.414 1.414L4 10.414V17a1 1 0 001 1h2a1 1 0 001-1v-2a1 1 0 011-1h2a1 1 0 011 1v2a1 1 0 001 1h2a1 1 0 001-1v-6.586l.293.293a1 1 0 001.414-1.414l-7-7z",
]);

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

// ── Clipboard ───────────────────────────────────────────
export const IconClipboardCopy = icon([
  "M8 2a1 1 0 000 2h2a1 1 0 100-2H8z",
  "M3 5a2 2 0 012-2 3 3 0 003 3h2a3 3 0 003-3 2 2 0 012 2v6h-4.586l1.293-1.293a1 1 0 00-1.414-1.414l-3 3a1 1 0 000 1.414l3 3a1 1 0 001.414-1.414L10.414 13H15v3a2 2 0 01-2 2H5a2 2 0 01-2-2V5zM15 11h2a1 1 0 110 2h-2v-2z",
]);

export const IconClipboardPaste = icon([
  "M8 3a1 1 0 011-1h2a1 1 0 110 2H9a1 1 0 01-1-1z",
  "M6 3a2 2 0 00-2 2v11a2 2 0 002 2h8a2 2 0 002-2V5a2 2 0 00-2-2 3 3 0 01-3 3H9a3 3 0 01-3-3z",
]);

// ── Misc ────────────────────────────────────────────────
export const IconCloudUpload = iconStroke(
  "M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12",
);

export const IconFolderOutline = iconStroke(
  "M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z",
);
