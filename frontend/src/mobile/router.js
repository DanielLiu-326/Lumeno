import { createRouter, createWebHistory } from "vue-router";
import HomeScreen from "./screens/HomeScreen.vue";
import AlbumsScreen from "./screens/AlbumsScreen.vue";
import FilesScreen from "./screens/FilesScreen.vue";
import MeScreen from "./screens/MeScreen.vue";

export function albumLocation(dir) {
  const parts = String(dir || "").split("/").filter(Boolean);
  return { name: "album", params: { dirName: parts } };
}

const AI_TOOLS = new Set(["faces", "duplicates", "search"]);

export function albumAiLocation(dir, section) {
  const parts = String(dir || "").split("/").filter(Boolean);
  const next = AI_TOOLS.has(section) ? section : "faces";
  return { name: "album-ai", params: { dirName: parts }, query: { section: next } };
}

export function albumAiSection(value) {
  const next = String(value || "");
  return AI_TOOLS.has(next) ? next : "";
}

export function albumFaceLocation(dir, target = {}) {
  const parts = String(dir || "").split("/").filter(Boolean);
  const query = {};
  if (target.person) query.person = String(target.person);
  if (target.cluster) query.cluster = String(target.cluster);
  return { name: "album-face", params: { dirName: parts }, query };
}

export function albumFaceCenterLocation(dir, target = {}) {
  const parts = String(dir || "").split("/").filter(Boolean);
  return {
    name: "album-face-center",
    params: { dirName: parts },
    query: {
      person: String(target.person || ""),
      center: String(target.center || ""),
    },
  };
}

export function albumViewLocation(dir) {
  const parts = String(dir || "").split("/").filter(Boolean);
  return { name: "album-view", params: { dirName: parts } };
}

export const MAIN_TAB_NAMES = new Set(["home", "albums", "files", "me"]);

export function isMainTabRoot(route) {
  return MAIN_TAB_NAMES.has(route?.name);
}

export function createMobileRouter() {
  return createRouter({
    history: createWebHistory(),
    routes: [
      { path: "/", name: "home", component: HomeScreen, meta: { tab: "home" } },
      { path: "/albums", name: "albums", component: AlbumsScreen, meta: { tab: "albums" } },
      { path: "/albums/:dirName+", name: "album", component: AlbumsScreen, meta: { tab: "albums" } },
      { path: "/album-ai/:dirName+", name: "album-ai", component: AlbumsScreen, meta: { tab: "albums" } },
      { path: "/album-face/:dirName+", name: "album-face", component: AlbumsScreen, meta: { tab: "albums" } },
      { path: "/album-face-center/:dirName+", name: "album-face-center", component: AlbumsScreen, meta: { tab: "albums" } },
      { path: "/album-view/:dirName+", name: "album-view", component: AlbumsScreen, meta: { tab: "albums" } },
      { path: "/files", name: "files", component: FilesScreen, meta: { tab: "files" } },
      { path: "/me", name: "me", component: MeScreen, meta: { tab: "me" } },
      { path: "/me/:section", name: "me-settings", component: MeScreen, meta: { tab: "me" } },
      { path: "/:pathMatch(.*)*", redirect: "/" },
    ],
  });
}
