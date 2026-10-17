import { locale, t } from "../i18n/index.js";

export const DEFAULT_VIDEO_ENCODE_ORDER = ["av1", "hevc", "h264"];

let encodeOrder = [...DEFAULT_VIDEO_ENCODE_ORDER];
const infoCache = new Map();

export function normalizeVideoEncodeOrder(values) {
  const order = [];
  for (const value of values || []) {
    const codec = String(value || "").trim().toLowerCase();
    if ((codec === "av1" || codec === "hevc" || codec === "h264") && !order.includes(codec)) {
      order.push(codec);
    }
  }
  for (const codec of DEFAULT_VIDEO_ENCODE_ORDER) {
    if (!order.includes(codec)) order.push(codec);
  }
  return order;
}

export function setVideoEncodeOrder(values) {
  encodeOrder = normalizeVideoEncodeOrder(values);
  return encodeOrder.slice();
}

export function getVideoEncodeOrder() {
  return encodeOrder.slice();
}

export function codecLabel(codec) {
  if (codec === "h264") return "H.264";
  if (codec === "hevc") return "HEVC";
  if (codec === "av1") return "AV1";
  return String(codec || "").toUpperCase();
}

function canPlay(type) {
  if (typeof document === "undefined") return false;
  const video = document.createElement("video");
  return video.canPlayType(type) !== "";
}

let av1Support;
let hevcSupport;

export function browserCanPlayAv1() {
  if (av1Support === undefined) {
    av1Support = canPlay('video/mp4; codecs="av01.0.05M.08"') || canPlay('video/webm; codecs="av01.0.05M.08"');
  }
  return av1Support;
}

export function browserCanPlayHevc() {
  if (hevcSupport === undefined) {
    hevcSupport = canPlay('video/mp4; codecs="hvc1.1.6.L93.B0"') || canPlay('video/mp4; codecs="hev1.1.6.L93.B0"');
  }
  return hevcSupport;
}

function chroma420(pix) {
  const value = String(pix || "").toLowerCase();
  return value.startsWith("yuv420") || value.startsWith("yuvj420");
}

function audioOk(codec) {
  const value = String(codec || "").toLowerCase();
  if (!value || value === "none") return true;
  return value === "aac" || value === "mp3" || value === "opus" || value === "vorbis";
}

export function browserCanPlaySource(source) {
  if (!source) return false;
  const depth = Number(source.bit_depth) || 8;
  const pix = source.pix_fmt || "";
  const audio = source.audio_codec || "";
  if (source.video_codec === "h264") {
    const profile = String(source.profile || "").toLowerCase();
    const profileOk = !profile
      || profile === "baseline"
      || profile === "constrained baseline"
      || profile === "main"
      || profile === "high";
    return depth <= 8 && chroma420(pix) && profileOk && audioOk(audio);
  }
  if (source.video_codec === "av1") {
    return (depth === 8 || depth === 10) && chroma420(pix) && audioOk(audio) && browserCanPlayAv1();
  }
  if (source.video_codec === "hevc") return browserCanPlayHevc();
  return false;
}

export function browserCanPlayEncode(codec) {
  if (codec === "h264") return true;
  if (codec === "av1") return browserCanPlayAv1();
  if (codec === "hevc") return browserCanPlayHevc();
  return false;
}

export function planPlayback(info, order = getVideoEncodeOrder()) {
  const source = info?.source;
  const original = {
    mode: "original",
    from: source?.label || "",
    to: "",
    badge: t("detail.videoOriginal"),
  };
  if (!source) {
    return {
      mode: "ask",
      codec: "h264",
      from: "",
      to: "H.264",
      badge: t("detail.videoOriginal"),
      ask: t("detail.videoTranscodeAsk", { codec: "H.264" }),
    };
  }
  if (browserCanPlaySource(source)) return original;
  const encodes = info.encodes || [];
  for (const codec of normalizeVideoEncodeOrder(order)) {
    const found = encodes.find((item) => item.codec === codec && item.ready && item.url);
    if (!found || !browserCanPlayEncode(codec)) continue;
    const to = found.label || codecLabel(codec);
    return {
      mode: "encode",
      codec,
      url: found.url,
      from: source.label,
      to,
      badge: `${source.label} → ${to}`,
    };
  }
  const target = normalizeVideoEncodeOrder(order).find((codec) => browserCanPlayEncode(codec)) || "h264";
  const label = codecLabel(target);
  return {
    mode: "ask",
    codec: target,
    from: source.label,
    to: label,
    badge: t("detail.videoOriginal"),
    ask: t("detail.videoTranscodeAsk", { codec: label }),
  };
}

export function videoBadgeText(info) {
  if (!info?.source) return "";
  return planPlayback(info).badge || "";
}

async function apiJson(url, options = {}) {
  const response = await fetch(url, {
    credentials: "include",
    headers: {
      "Accept-Language": locale.value,
      ...(options.body ? { "Content-Type": "application/json" } : {}),
      ...(options.headers || {}),
    },
    ...options,
  });
  const payload = await response.json();
  if (!payload.success) {
    const error = new Error(payload.message || t("msg.requestFailed"));
    error.status = response.status;
    throw error;
  }
  return payload;
}

function infoKey(path, owner) {
  return `${owner || ""}:${path}`;
}

export function videoInfoUrl(path, owner) {
  const encoded = encodeURIComponent(path).replace(/%2F/g, "/");
  const query = owner ? `?owner=${encodeURIComponent(owner)}` : "";
  return `/api/albums/video_info/${encoded}${query}`;
}

export async function loadVideoInfo(path, owner, { refresh = false } = {}) {
  const key = infoKey(path, owner);
  if (!refresh && infoCache.has(key)) return infoCache.get(key);
  const payload = await apiJson(videoInfoUrl(path, owner));
  infoCache.set(key, payload.data);
  return payload.data;
}

export async function startVideoEncode(path, owner, codec) {
  const payload = await apiJson("/api/albums/video_encode", {
    method: "POST",
    body: JSON.stringify({ path, owner: owner || undefined, codec }),
  });
  return payload.data;
}

export function watchVideoEncode(path, owner, codec, taskId, isCurrent, onReady, onFail) {
  let stopped = false;
  let attempts = 0;
  const timer = window.setInterval(async () => {
    if (stopped || !isCurrent() || attempts > 450) {
      window.clearInterval(timer);
      if (!stopped && isCurrent()) onFail?.();
      return;
    }
    attempts += 1;
    try {
      const info = await loadVideoInfo(path, owner, { refresh: true });
      const ready = (info.encodes || []).some((item) => item.codec === codec && item.ready);
      if (ready && isCurrent()) {
        window.clearInterval(timer);
        onReady(planPlayback(info));
        return;
      }
      if (!taskId) return;
      const tasks = await apiJson("/api/get_task_list/");
      const task = tasks.data?.[taskId];
      if (task && (task.status === 4 || task.status === 5) && isCurrent()) {
        window.clearInterval(timer);
        onFail?.(task.error_msg);
      }
    } catch {
      /* keep waiting while the task is still running */
    }
  }, 2000);
  return () => {
    stopped = true;
    window.clearInterval(timer);
  };
}
