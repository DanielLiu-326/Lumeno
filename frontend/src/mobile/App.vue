<script setup>
import { computed, onBeforeUnmount, onMounted, provide, proxyRefs, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import ConflictModal from "../components/ConflictModal.vue";
import { useAlbumApp } from "../composables/useAlbumApp";
import { t, locale } from "../i18n/index.js";
import { applyAccent } from "../utils/accent.js";
import { normalizeDiscoveryPhoto } from "../utils/photoDetail.js";
import { scheduleSafeAreaSync } from "../utils/safeArea.js";
import { layoutViewportSize } from "../utils/uiScale.js";
import { withThemeTransition } from "../utils/themeTransition.js";
import { wallpaperLayerState } from "../utils/wallpaper.js";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "./context.js";
import { discardOverlayStack } from "./useOverlayHistory.js";
import { isMainTabRoot } from "./router.js";
import TabBar from "./layouts/TabBar.vue";
import AlbumScreen from "./screens/AlbumScreen.vue";
import AiToolsScreen from "./screens/AiToolsScreen.vue";
import FaceDetailScreen from "./screens/FaceDetailScreen.vue";
import FaceCenterScreen from "./screens/FaceCenterScreen.vue";
import ViewSettingsScreen from "./screens/ViewSettingsScreen.vue";
import LoginScreen from "./screens/LoginScreen.vue";
import MeSettingsScreen from "./screens/MeSettingsScreen.vue";
import NotifySheet from "./sheets/NotifySheet.vue";
import PhotoViewer from "./sheets/PhotoViewer.vue";
import TaskSheet from "./sheets/TaskSheet.vue";
import { useBannerSwipe } from "./useBannerSwipe.js";

const app = proxyRefs(useAlbumApp());
provide(ALBUM_APP_KEY, app);

const route = useRoute();
const router = useRouter();
const theme = ref("light");
const toastVisible = ref(false);
const toastMessage = ref("");
const taskListOpen = ref(false);
const notificationOpen = ref(false);
const notifications = ref([]);
let toastTimer = null;

function hideToast() {
  toastVisible.value = false;
  if (toastTimer) {
    clearTimeout(toastTimer);
    toastTimer = null;
  }
}

function armToastTimer() {
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { hideToast(); }, 3200);
}

const {
  dragging: toastDragging,
  exiting: toastExiting,
  style: toastStyle,
  onPointerDown: onToastPointerDown,
  onClick: onToastSwipeClick,
  clear: clearToastSwipe,
} = useBannerSwipe({
  onDismiss: hideToast,
  onDragStart() {
    if (toastTimer) {
      clearTimeout(toastTimer);
      toastTimer = null;
    }
  },
  onSettle() {
    if (toastVisible.value) armToastTimer();
  },
});

const THEME_COLOR = { light: "#f4efe7", dark: "#0f172a" };
const isDark = computed(() => theme.value === "dark");
const wallpaperLayer = computed(() => wallpaperLayerState(app.userWallpaper, isDark.value));
const isAlbumRoute = computed(() => route.name === "album");
const isAiRoute = computed(() => route.name === "album-ai");
const isFaceRoute = computed(() => route.name === "album-face");
const isFaceCenterRoute = computed(() => route.name === "album-face-center");
const isViewRoute = computed(() => route.name === "album-view");
const isSettingsRoute = computed(() => route.name === "me-settings");
const faceSurfaceOpen = computed(() => isFaceRoute.value || isFaceCenterRoute.value);
const aiSurfaceOpen = computed(() => isAiRoute.value || faceSurfaceOpen.value);
const albumSurfaceOpen = computed(() => isAlbumRoute.value || aiSurfaceOpen.value || isViewRoute.value);
const settingsLayerOpen = ref(isSettingsRoute.value);
const albumLayerOpen = ref(albumSurfaceOpen.value);
const aiLayerOpen = ref(aiSurfaceOpen.value);
const faceLayerOpen = ref(faceSurfaceOpen.value);
const faceCenterLayerOpen = ref(isFaceCenterRoute.value);
const viewLayerOpen = ref(isViewRoute.value);
const settingsInstant = ref(false);
const albumInstant = ref(false);
const aiInstant = ref(false);
const faceInstant = ref(false);
const faceCenterInstant = ref(false);
const viewInstant = ref(false);
const settingsSettled = ref(false);
const albumSettled = ref(false);
const aiSettled = ref(false);
const faceSettled = ref(false);
const faceCenterSettled = ref(false);
const viewSettled = ref(false);
const settingsLayerRef = ref(null);
const albumLayerRef = ref(null);
const aiLayerRef = ref(null);
const faceLayerRef = ref(null);
const faceCenterLayerRef = ref(null);
const viewLayerRef = ref(null);
let animatedPushClose = false;

function markPushClose() {
  animatedPushClose = true;
}

function consumeAnimatedPushClose() {
  const next = animatedPushClose;
  animatedPushClose = false;
  return next;
}

function clearInstantClose() {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      settingsInstant.value = false;
      albumInstant.value = false;
      aiInstant.value = false;
      faceInstant.value = false;
      faceCenterInstant.value = false;
      viewInstant.value = false;
    });
  });
}

function snapLayerClosed(el) {
  if (!el) return;
  if (el === settingsLayerRef.value) settingsSettled.value = false;
  else if (el === albumLayerRef.value) albumSettled.value = false;
  else if (el === aiLayerRef.value) aiSettled.value = false;
  else if (el === faceLayerRef.value) faceSettled.value = false;
  else if (el === faceCenterLayerRef.value) faceCenterSettled.value = false;
  else if (el === viewLayerRef.value) viewSettled.value = false;
  el.classList.remove("is-settled");
  el.classList.add("is-instant");
  el.classList.remove("is-open");
}

function settleLayer(el) {
  if (!el || !el.classList.contains("is-open") || el.classList.contains("is-instant")) return;
  if (el === settingsLayerRef.value) settingsSettled.value = true;
  else if (el === albumLayerRef.value) albumSettled.value = true;
  else if (el === aiLayerRef.value) aiSettled.value = true;
  else if (el === faceLayerRef.value) faceSettled.value = true;
  else if (el === faceCenterLayerRef.value) faceCenterSettled.value = true;
  else if (el === viewLayerRef.value) viewSettled.value = true;
  requestAnimationFrame(() => repaintMobileApp());
}

function repaintMobileApp() {
  const app = document.querySelector(".mobile-app");
  if (!app) return;
  app.classList.remove("is-repaint");
  void app.offsetWidth;
  app.classList.add("is-repaint");
  window.setTimeout(() => app.classList.remove("is-repaint"), 40);
  app.addEventListener("animationend", () => app.classList.remove("is-repaint"), { once: true });
}

// iOS rasters the first screen lazily while the boot splash still covers it and
// keeps that low-quality raster afterwards (reads as a gaussian blur on the top
// region, including the strip under the status bar). An edge-swipe back clears it
// because it re-inserts the screen node, so do the same here: take the screen and
// the notch cap out of the DOM, flush layout, and put them back in place. A plain
// repaint or a `display: contents` swap is not enough on current WebKit.
function sharpenScreenBoxes() {
  if (document.querySelector(".mobile-push-layer.is-open")) return;
  if (document.querySelector("[class*='page-enter-active'], [class*='page-leave-active']")) return;
  const host = document.querySelector(".mobile-page-host");
  const cap = document.querySelector(".mobile-notch-cap");
  const targets = [];
  if (host) targets.push(...host.children);
  if (cap) targets.push(cap);
  const moves = [];
  for (const el of targets) {
    if (!(el instanceof HTMLElement) || !el.isConnected || !el.parentNode) continue;
    moves.push({ el, parent: el.parentNode, next: el.nextSibling });
  }
  if (!moves.length) return;
  for (const { el, parent } of moves) parent.removeChild(el);
  void document.body.offsetHeight;
  for (const { el, parent, next } of moves) {
    if (next && next.parentNode === parent) parent.insertBefore(el, next);
    else parent.appendChild(el);
  }
}

function sharpenAndRepaintMobileApp() {
  requestAnimationFrame(() => {
    sharpenScreenBoxes();
    repaintMobileApp();
  });
}

function onLumenoBooted() {
  // The first screen may still be painting when boot ends; repeat the rebuild a
  // few times instead of trusting a single 480 ms timer.
  for (const delay of [120, 480, 1200]) {
    window.setTimeout(sharpenScreenBoxes, delay);
  }
}

// iOS re-rasters the page when a standalone app returns from the background or
// restores a tab, and the restored raster keeps the soft top region. Rebuild the
// screen boxes on both events instead of only after boot.
function onPageShow() {
  syncAppViewport();
  sharpenAndRepaintMobileApp();
}

function onVisibilityChange() {
  if (document.visibilityState === "visible") sharpenAndRepaintMobileApp();
}

function releaseLayer(el, settled) {
  if (!settled.value && !el?.classList.contains("is-settled")) return;
  settled.value = false;
  el?.classList.remove("is-settled");
  if (el) void el.offsetWidth;
}

function onLayerTransitionEnd(event) {
  if (event.target !== event.currentTarget || event.propertyName !== "transform") return;
  const layer = event.currentTarget;
  if (layer instanceof Element && !layer.classList.contains("is-open")) {
    // A closing layer re-exposes the screen behind it; WebKit can keep a soft
    // raster of that region, which reads as a gaussian blur around the top bar.
    requestAnimationFrame(() => {
      sharpenScreenBoxes();
      repaintMobileApp();
    });
    return;
  }
  settleLayer(layer);
}

function armLayerSettle(openRef, elRef) {
  window.setTimeout(() => {
    if (openRef.value) settleLayer(elRef.value);
  }, 420);
}

const iosSwipeBack = /iP(hone|ad|od)/.test(navigator.userAgent || "");
let edgeSwipe = null;

function onEdgeTouchStart(event) {
  if (!iosSwipeBack || event.touches?.length !== 1) return;
  const touch = event.touches[0];
  if (touch.clientX > 28) return;
  edgeSwipe = {
    x: touch.clientX,
    y: touch.clientY,
    path: `${location.pathname}${location.search}`,
    dragging: false,
  };
}

function onEdgeTouchMove(event) {
  if (!edgeSwipe || event.touches?.length !== 1) return;
  const touch = event.touches[0];
  const dx = touch.clientX - edgeSwipe.x;
  const dy = touch.clientY - edgeSwipe.y;
  if (edgeSwipe.dragging) return;
  if (Math.abs(dx) < 10 && Math.abs(dy) < 10) return;
  if (dx <= 0 || Math.abs(dy) > Math.abs(dx)) {
    edgeSwipe = null;
    return;
  }
  edgeSwipe.dragging = true;
}

function sharpenOpenLayers() {
  const layers = [
    [settingsLayerOpen, settingsSettled, settingsLayerRef],
    [albumLayerOpen, albumSettled, albumLayerRef],
    [aiLayerOpen, aiSettled, aiLayerRef],
    [faceLayerOpen, faceSettled, faceLayerRef],
    [faceCenterLayerOpen, faceCenterSettled, faceCenterLayerRef],
    [viewLayerOpen, viewSettled, viewLayerRef],
  ];
  const openLayers = [];
  for (const [open, settled, elRef] of layers) {
    const el = elRef.value;
    if (!open.value || !el?.classList.contains("is-open")) continue;
    settled.value = false;
    el.classList.remove("is-settled");
    void el.offsetWidth;
    openLayers.push([open, settled, el]);
  }
  requestAnimationFrame(() => {
    for (const [open, settled, el] of openLayers) {
      if (!open.value || !el.classList.contains("is-open") || el.classList.contains("is-instant")) continue;
      settled.value = true;
    }
    requestAnimationFrame(() => repaintMobileApp());
  });
}

function onEdgeTouchEnd() {
  if (!edgeSwipe?.dragging) {
    edgeSwipe = null;
    return;
  }
  const path = edgeSwipe.path;
  edgeSwipe = null;
  window.setTimeout(() => {
    if (`${location.pathname}${location.search}` !== path) return;
    sharpenOpenLayers();
  }, 420);
}

function syncPushLayersFromLocation() {
  const path = window.location.pathname;
  const settings = /^\/me\/[^/]+\/?$/.test(path);
  const faceCenter = /^\/album-face-center\//.test(path);
  const face = /^\/album-face\//.test(path) || faceCenter;
  const ai = /^\/album-ai\//.test(path) || face;
  const view = /^\/album-view\//.test(path);
  const album = /^\/albums\/.+/.test(path) || ai || view;
  const animated = consumeAnimatedPushClose();
  if (settingsLayerOpen.value && !settings) {
    if (!animated) {
      settingsInstant.value = true;
      snapLayerClosed(settingsLayerRef.value);
    }
    settingsLayerOpen.value = false;
  } else if (settings) {
    settingsLayerOpen.value = true;
  }
  if (aiLayerOpen.value && !ai) {
    if (!animated) {
      aiInstant.value = true;
      snapLayerClosed(aiLayerRef.value);
    }
    aiLayerOpen.value = false;
  } else if (ai) {
    aiLayerOpen.value = true;
  }
  if (faceLayerOpen.value && !face) {
    if (!animated) {
      faceInstant.value = true;
      snapLayerClosed(faceLayerRef.value);
    }
    faceLayerOpen.value = false;
  } else if (face) {
    faceLayerOpen.value = true;
  }
  if (faceCenterLayerOpen.value && !faceCenter) {
    if (!animated) {
      faceCenterInstant.value = true;
      snapLayerClosed(faceCenterLayerRef.value);
    }
    faceCenterLayerOpen.value = false;
  } else if (faceCenter) {
    faceCenterLayerOpen.value = true;
  }
  if (viewLayerOpen.value && !view) {
    if (!animated) {
      viewInstant.value = true;
      snapLayerClosed(viewLayerRef.value);
    }
    viewLayerOpen.value = false;
  } else if (view) {
    viewLayerOpen.value = true;
  }
  if (albumLayerOpen.value && !album) {
    if (!animated) {
      albumInstant.value = true;
      snapLayerClosed(albumLayerRef.value);
    }
    albumLayerOpen.value = false;
  } else if (album) {
    albumLayerOpen.value = true;
  }
  if (settingsInstant.value || albumInstant.value || aiInstant.value || faceInstant.value || faceCenterInstant.value || viewInstant.value) clearInstantClose();
}

watch(albumSurfaceOpen, (open, wasOpen) => {
  if (wasOpen && !open) discardOverlayStack();
  if (open) albumLayerOpen.value = true;
  else if (albumLayerOpen.value) albumLayerOpen.value = false;
});

watch(aiSurfaceOpen, (open) => {
  if (open) aiLayerOpen.value = true;
  else if (aiLayerOpen.value) aiLayerOpen.value = false;
});

watch(faceSurfaceOpen, (open) => {
  if (open) faceLayerOpen.value = true;
  else if (faceLayerOpen.value) faceLayerOpen.value = false;
});

watch(isFaceCenterRoute, (open) => {
  if (open) faceCenterLayerOpen.value = true;
  else if (faceCenterLayerOpen.value) faceCenterLayerOpen.value = false;
});

watch(isViewRoute, (open) => {
  if (open) viewLayerOpen.value = true;
  else if (viewLayerOpen.value) viewLayerOpen.value = false;
});

watch(isSettingsRoute, (open) => {
  if (open) settingsLayerOpen.value = true;
  else if (settingsLayerOpen.value) settingsLayerOpen.value = false;
});

watch(albumLayerOpen, (open, wasOpen) => {
  if (open && !wasOpen) armLayerSettle(albumLayerOpen, albumLayerRef);
  if (wasOpen && !open) releaseLayer(albumLayerRef.value, albumSettled);
});
watch(aiLayerOpen, (open, wasOpen) => {
  if (open && !wasOpen) armLayerSettle(aiLayerOpen, aiLayerRef);
  if (wasOpen && !open) releaseLayer(aiLayerRef.value, aiSettled);
});
watch(faceLayerOpen, (open, wasOpen) => {
  if (open && !wasOpen) armLayerSettle(faceLayerOpen, faceLayerRef);
  if (wasOpen && !open) releaseLayer(faceLayerRef.value, faceSettled);
});
watch(faceCenterLayerOpen, (open, wasOpen) => {
  if (open && !wasOpen) armLayerSettle(faceCenterLayerOpen, faceCenterLayerRef);
  if (wasOpen && !open) releaseLayer(faceCenterLayerRef.value, faceCenterSettled);
});
watch(viewLayerOpen, (open, wasOpen) => {
  if (open && !wasOpen) armLayerSettle(viewLayerOpen, viewLayerRef);
  if (wasOpen && !open) releaseLayer(viewLayerRef.value, viewSettled);
});
watch(settingsLayerOpen, (open, wasOpen) => {
  if (open && !wasOpen) armLayerSettle(settingsLayerOpen, settingsLayerRef);
  if (wasOpen && !open) releaseLayer(settingsLayerRef.value, settingsSettled);
});

function settleOpenLayers() {
  if (settingsLayerOpen.value) settingsSettled.value = true;
  if (albumLayerOpen.value) albumSettled.value = true;
  if (aiLayerOpen.value) aiSettled.value = true;
  if (faceLayerOpen.value) faceSettled.value = true;
  if (faceCenterLayerOpen.value) faceCenterSettled.value = true;
  if (viewLayerOpen.value) viewSettled.value = true;
}

const activeTasks = computed(() => app.tasks.filter((task) => [0, 1, 2].includes(task.status)));
const activeTaskCount = computed(() => activeTasks.value.length);
const unreadNotificationCount = computed(() => notifications.value.filter((item) => !item.read).length);

watch(() => app.userAccent, (next) => applyAccent(next), { immediate: true });
watch(taskListOpen, (open) => app.setTaskListExpanded(open));

const ui = reactive({
  isDark,
  albumPanelRef: null,
  viewerPhoto: null,
  viewerHasPrev: false,
  viewerHasNext: false,
  viewerPrevPhoto: null,
  viewerNextPhoto: null,
  viewerIndexLabel: "",
  activeTaskCount,
  unreadNotificationCount,
  toggleTheme,
  openTasks,
  openNotify,
  openDiscoveryPhoto,
  markPushClose,
});
provide(MOBILE_UI_KEY, ui);

const TAB_INDEX = { home: 0, albums: 1, files: 2, me: 3 };
const pageTransition = ref("");
let historyPop = false;
let skipNextTransition = false;

function locationFullPath() {
  return `${window.location.pathname}${window.location.search}` || "/";
}

function normalizePath(path) {
  const value = String(path || "/");
  if (value.length > 1 && value.endsWith("/")) return value.slice(0, -1);
  return value || "/";
}

function onHistoryPop() {
  syncPushLayersFromLocation();
  historyPop = normalizePath(locationFullPath()) !== normalizePath(router.currentRoute.value.fullPath);
}

function transitionFor(to, from) {
  if (historyPop) {
    historyPop = false;
    return "";
  }
  if (!from?.name || to.name === from.name) return "";
  if (to.name === "album" || from.name === "album") return "";
  if (to.name === "album-ai" || from.name === "album-ai") return "";
  if (to.name === "album-face" || from.name === "album-face") return "";
  if (to.name === "album-face-center" || from.name === "album-face-center") return "";
  if (to.name === "album-view" || from.name === "album-view") return "";
  if (to.name === "me-settings" || from.name === "me-settings") return "";
  const fromTab = TAB_INDEX[from.meta?.tab];
  const toTab = TAB_INDEX[to.meta?.tab];
  if (fromTab == null || toTab == null || fromTab === toTab) return "mobile-page-fade";
  return toTab > fromTab ? "mobile-page-tab-next" : "mobile-page-tab-prev";
}

router.beforeEach((to, from) => {
  const pop = historyPop;
  if (pop && isMainTabRoot(from) && from.name !== "home" && to.name !== "home") {
    historyPop = false;
    skipNextTransition = true;
    pageTransition.value = "";
    return { name: "home", replace: true };
  }
  if (skipNextTransition) {
    skipNextTransition = false;
    historyPop = false;
    pageTransition.value = "";
    return;
  }
  pageTransition.value = transitionFor(to, from);
});

const hideTabBar = computed(() => Boolean(route.meta?.hideTabBar) || !!ui.viewerPhoto);

function applyThemeColor(nextTheme) {
  const color = nextTheme === "dark" ? THEME_COLOR.dark : THEME_COLOR.light;
  let meta = document.querySelector('meta[name="theme-color"]');
  if (!meta) {
    meta = document.createElement("meta");
    meta.setAttribute("name", "theme-color");
    document.head.appendChild(meta);
  }
  meta.setAttribute("content", color);
  document.documentElement.style.backgroundColor = color;
  document.body.style.backgroundColor = color;
}

function applyTheme(nextTheme, animate = false) {
  withThemeTransition(() => {
    theme.value = nextTheme;
    document.documentElement.classList.toggle("dark", nextTheme === "dark");
    localStorage.setItem("album-theme", nextTheme);
    applyThemeColor(nextTheme);
  }, { animate });
}

function toggleTheme() {
  applyTheme(isDark.value ? "light" : "dark", true);
}

let viewportSettleTimer = 0;

function syncAppViewport(settle = true) {
  const { height, top } = layoutViewportSize();
  document.documentElement.style.setProperty("--app-height", `${height}px`);
  document.documentElement.style.setProperty("--app-top", `${top}px`);
  scheduleSafeAreaSync();
  if (!settle) return;
  // iOS can report a momentary short innerHeight while bars or the keyboard
  // settle; re-measure once so the app (tab bar, sheets) fills the bottom.
  window.clearTimeout(viewportSettleTimer);
  viewportSettleTimer = window.setTimeout(() => syncAppViewport(false), 350);
}

function openTasks() {
  notificationOpen.value = false;
  taskListOpen.value = true;
}

function openNotify() {
  taskListOpen.value = false;
  notificationOpen.value = true;
  notifications.value = notifications.value.map((item) => ({ ...item, read: true }));
}

function closePhotoViewer() {
  ui.albumPanelRef?.closeDetail?.({ force: true });
  ui.viewerPhoto = null;
}

// WebKit keeps a soft raster of the swipe track while the viewer rests; the
// delayed repaint drops it so the top bar does not read as blurred.
function onViewerNav(dir) {
  if (dir > 0) ui.albumPanelRef?.goDetailNext?.();
  else ui.albumPanelRef?.goDetailPrev?.();
  window.setTimeout(() => {
    sharpenScreenBoxes();
    repaintMobileApp();
  }, 360);
}

watch(() => ui.viewerPhoto, (photo) => {
  if (photo) return;
  requestAnimationFrame(() => {
    sharpenScreenBoxes();
    repaintMobileApp();
  });
});

function onViewerSave(payload) {
  app.updatePhotoMeta(payload);
}

function onViewerDelete(photo) {
  if (!photo?.path) return;
  app.deleteAlbumPhotos([photo.path]);
  closePhotoViewer();
}

function currentAlbumWritable() {
  const dir = app.currentAlbum;
  if (!dir) return false;
  if (!String(dir).includes("/")) return true;
  const album = (app.sharedAlbums || []).find((item) => item.dir_name === dir);
  return !!album?.can_write;
}

function onViewerSetCover(photo) {
  if (!photo?.path || !currentAlbumWritable()) return;
  app.updateAlbum({ dirName: app.currentAlbum, cover: photo.path });
}

function onViewerCopy({ photoPath, sourceAlbum, targetAlbum }) {
  app.copyPhotoToAlbum({ photoPath, sourceAlbum, targetAlbum });
}

function onViewerSimilar(photo) {
  closePhotoViewer();
  app.findSimilarPhotos(photo);
}

function openDiscoveryPhoto({ photo, navPhotos, fullscreen = true }) {
  const normalized = normalizeDiscoveryPhoto(photo);
  if (!normalized) return;
  ui.albumPanelRef?.openPhotoDetail(normalized, {
    navPhotos: (navPhotos || []).map(normalizeDiscoveryPhoto).filter(Boolean),
    fullscreen,
  });
}

watch(
  () => app.message.sequence,
  () => {
    const text = app.message.text;
    if (!text) return;
    notifications.value = [{
      id: Date.now() + Math.random(),
      text,
      kind: app.message.kind,
      createdAt: new Date().toLocaleTimeString(locale.value === "zh" ? "zh-CN" : "en-US", { hour12: false }),
      read: false,
    }, ...notifications.value].slice(0, 40);
    toastMessage.value = text;
    toastVisible.value = true;
    clearToastSwipe();
    armToastTimer();
  },
);

watch(
  () => app.loggedIn,
  async (loggedIn) => {
    if (!loggedIn) {
      if (route.path !== "/") router.replace("/");
      return;
    }
    await app.refreshAlbums();
    await app.refreshDashboard();
  },
);

watch(
  () => route.name,
  (name, prev) => {
    if (!app.loggedIn) return;
    if (name === "albums") {
      if (prev === "album") return;
      app.refreshAlbums();
    } else if (name === "home") app.refreshDashboard();
    else if (name === "me" || name === "me-settings") app.refreshAiStatus();
  },
);

function preventBrowserPageZoom(event) {
  if (event.touches && event.touches.length > 1) event.preventDefault();
}

function preventBrowserGesture(event) {
  event.preventDefault();
}

function isEditableTarget(target) {
  return target instanceof Element && Boolean(
    target.closest("input, textarea, select, [contenteditable='true']"),
  );
}

function preventSelectStart(event) {
  if (isEditableTarget(event.target)) return;
  event.preventDefault();
}

function preventImageDrag(event) {
  if (event.target instanceof HTMLImageElement) event.preventDefault();
}

onMounted(() => {
  const saved = localStorage.getItem("album-theme");
  applyTheme(saved === "dark" ? "dark" : "light");
  syncAppViewport();
  for (const el of [settingsLayerRef.value, albumLayerRef.value, aiLayerRef.value, faceLayerRef.value, faceCenterLayerRef.value, viewLayerRef.value]) {
    el?.addEventListener("transitionend", onLayerTransitionEnd);
  }
  settleOpenLayers();
  window.addEventListener("touchstart", onEdgeTouchStart, { passive: true });
  window.addEventListener("touchmove", onEdgeTouchMove, { passive: true });
  window.addEventListener("touchend", onEdgeTouchEnd, { passive: true });
  window.addEventListener("touchcancel", onEdgeTouchEnd, { passive: true });
  document.addEventListener("touchmove", preventBrowserPageZoom, { passive: false });
  document.addEventListener("gesturestart", preventBrowserGesture, { passive: false });
  document.addEventListener("gesturechange", preventBrowserGesture, { passive: false });
  document.addEventListener("selectstart", preventSelectStart);
  document.addEventListener("dragstart", preventImageDrag);
  window.addEventListener("popstate", onHistoryPop, true);
  // The first screen is rastered while the splash still covers it; once the
  // splash is gone, rebuild its box so the top region gets a clean raster.
  window.addEventListener("lumeno:booted", onLumenoBooted, { once: true });
  if (!document.getElementById("boot-splash")) {
    window.setTimeout(sharpenScreenBoxes, 480);
  }
  window.addEventListener("resize", syncAppViewport);
  window.addEventListener("orientationchange", syncAppViewport);
  window.addEventListener("pageshow", onPageShow);
  document.addEventListener("visibilitychange", onVisibilityChange);
  window.addEventListener("focusin", syncAppViewport);
  window.addEventListener("focusout", syncAppViewport);
  window.visualViewport?.addEventListener("resize", syncAppViewport);
  window.visualViewport?.addEventListener("scroll", syncAppViewport);
});

onBeforeUnmount(() => {
  for (const el of [settingsLayerRef.value, albumLayerRef.value, aiLayerRef.value, faceLayerRef.value, faceCenterLayerRef.value, viewLayerRef.value]) {
    el?.removeEventListener("transitionend", onLayerTransitionEnd);
  }
  window.removeEventListener("touchstart", onEdgeTouchStart);
  window.removeEventListener("touchmove", onEdgeTouchMove);
  window.removeEventListener("touchend", onEdgeTouchEnd);
  window.removeEventListener("touchcancel", onEdgeTouchEnd);
  document.removeEventListener("touchmove", preventBrowserPageZoom);
  document.removeEventListener("gesturestart", preventBrowserGesture);
  document.removeEventListener("gesturechange", preventBrowserGesture);
  document.removeEventListener("selectstart", preventSelectStart);
  document.removeEventListener("dragstart", preventImageDrag);
  window.removeEventListener("popstate", onHistoryPop, true);
  window.removeEventListener("lumeno:booted", onLumenoBooted);
  window.removeEventListener("resize", syncAppViewport);
  window.removeEventListener("orientationchange", syncAppViewport);
  window.removeEventListener("pageshow", onPageShow);
  document.removeEventListener("visibilitychange", onVisibilityChange);
  window.removeEventListener("focusin", syncAppViewport);
  window.removeEventListener("focusout", syncAppViewport);
  window.visualViewport?.removeEventListener("resize", syncAppViewport);
  window.visualViewport?.removeEventListener("scroll", syncAppViewport);
  if (toastTimer) clearTimeout(toastTimer);
  window.clearTimeout(viewportSettleTimer);
});
</script>

<template>
  <LoginScreen
    v-if="!app.loggedIn"
    :is-dark="isDark"
    :busy-login="app.busy.login"
    :login-form="app.loginForm"
    @submit="app.login"
    @register="app.register"
  />

  <div
    v-else
    class="mobile-app"
    :class="isDark ? 'text-stone-100' : 'text-stone-900'"
  >
    <div class="pointer-events-none absolute inset-0" :style="wallpaperLayer.style"></div>
    <div class="mobile-notch-cap" aria-hidden="true"></div>
    <div class="mobile-page-host relative z-10 flex min-h-0 flex-1 flex-col overflow-hidden">
      <RouterView v-slot="{ Component, route: viewRoute }">
        <Transition :name="pageTransition" :css="Boolean(pageTransition)" @after-enter="sharpenAndRepaintMobileApp" @after-leave="sharpenAndRepaintMobileApp">
          <KeepAlive include="HomeScreen,AlbumsScreen,FilesScreen,MeScreen">
            <component :is="Component" :key="viewRoute.meta.tab || viewRoute.name" />
          </KeepAlive>
        </Transition>
      </RouterView>
    </div>

    <div
      ref="settingsLayerRef"
      class="mobile-push-layer mobile-settings-layer"
      :class="{ 'is-open': settingsLayerOpen, 'is-instant': settingsInstant, 'is-settled': settingsSettled }"
    >
      <div class="mobile-push-frame"><MeSettingsScreen /></div>
    </div>

    <div
      ref="albumLayerRef"
      class="mobile-push-layer mobile-album-layer"
      :class="{ 'is-open': albumLayerOpen, 'is-instant': albumInstant, 'is-settled': albumSettled }"
    >
      <div class="mobile-push-frame"><AlbumScreen /></div>
    </div>

    <div
      ref="aiLayerRef"
      class="mobile-push-layer mobile-ai-layer"
      :class="{ 'is-open': aiLayerOpen, 'is-instant': aiInstant, 'is-settled': aiSettled }"
    >
      <div class="mobile-push-frame"><AiToolsScreen /></div>
    </div>

    <div
      ref="faceLayerRef"
      class="mobile-push-layer mobile-face-layer"
      :class="{ 'is-open': faceLayerOpen, 'is-instant': faceInstant, 'is-settled': faceSettled }"
    >
      <div class="mobile-push-frame"><FaceDetailScreen /></div>
    </div>

    <div
      ref="faceCenterLayerRef"
      class="mobile-push-layer mobile-face-center-layer"
      :class="{ 'is-open': faceCenterLayerOpen, 'is-instant': faceCenterInstant, 'is-settled': faceCenterSettled }"
    >
      <div class="mobile-push-frame"><FaceCenterScreen /></div>
    </div>

    <div
      ref="viewLayerRef"
      class="mobile-push-layer mobile-view-layer"
      :class="{ 'is-open': viewLayerOpen, 'is-instant': viewInstant, 'is-settled': viewSettled }"
    >
      <div class="mobile-push-frame"><ViewSettingsScreen /></div>
    </div>

    <PhotoViewer
      :photo="ui.viewerPhoto"
      :is-dark="isDark"
      :format-size="app.formatSize"
      :can-delete="!!ui.albumPanelRef?.currentAlbumCanDelete"
      :can-write="ui.albumPanelRef?.currentAlbumCanWrite !== false"
      :albums="app.albums"
      :shared-albums="app.sharedAlbums"
      :current-album="app.currentAlbum"
      :has-prev="ui.viewerHasPrev"
      :has-next="ui.viewerHasNext"
      :prev-photo="ui.viewerPrevPhoto"
      :next-photo="ui.viewerNextPhoto"
      :index-label="ui.viewerIndexLabel"
      :available-tags="app.albumAvailableTags"
      @close="closePhotoViewer"
      @prev="onViewerNav(-1)"
      @next="onViewerNav(1)"
      @save="onViewerSave"
      @delete="onViewerDelete"
      @set-cover="onViewerSetCover"
      @copy-to-album="onViewerCopy"
      @find-similar="onViewerSimilar"
    />

    <TabBar v-if="!hideTabBar" />

    <ConflictModal
      :show="app.conflictModalVisible"
      :is-dark="isDark"
      @choose="app.resolveConflict"
      @cancel="app.cancelConflict"
    />

    <TaskSheet
      :show="taskListOpen"
      :is-dark="isDark"
      :tasks="app.tasks"
      @close="taskListOpen = false"
      @pause="app.pauseTask"
      @resume="app.resumeTask"
      @cancel="app.cancelTask"
      @clear-finished="app.clearFinishedTasks"
    />
    <NotifySheet
      :show="notificationOpen"
      :is-dark="isDark"
      :items="notifications"
      @close="notificationOpen = false"
      @clear="notifications = []"
    />

    <teleport to="body">
      <transition :name="toastExiting ? '' : 'mobile-toast'">
        <button
          v-if="toastVisible"
          type="button"
          class="mobile-toast app-frost app-card shadow-2xl"
          :class="{ 'is-dragging': toastDragging, 'is-exiting': toastExiting }"
          :style="toastStyle"
          @pointerdown="onToastPointerDown"
          @click="(event) => { if (onToastSwipeClick(event)) return; openNotify(); hideToast(); }"
        >
          <p class="truncate text-sm">{{ toastMessage }}</p>
          <p class="mt-1 text-xs opacity-60">{{ t("topbar.clickForNotifications") }}</p>
        </button>
      </transition>
    </teleport>
  </div>
</template>
