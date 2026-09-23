<script setup>
import { computed, inject } from "vue";
import { useRoute, useRouter } from "vue-router";
import { t } from "../../i18n/index.js";
import { MOBILE_UI_KEY } from "../context.js";
import { isMainTabRoot } from "../router.js";
import { IconAlbum, IconFolder, IconHome, IconUser } from "../../utils/icons";

const route = useRoute();
const router = useRouter();
const ui = inject(MOBILE_UI_KEY);

const tabs = computed(() => [
  { id: "home", to: "/", label: t("nav.home"), icon: IconHome },
  { id: "albums", to: "/albums", label: t("nav.albums"), icon: IconAlbum },
  { id: "files", to: "/files", label: t("nav.files"), icon: IconFolder },
  { id: "me", to: "/me", label: t("nav.me"), icon: IconUser },
]);

const activeTab = computed(() => route.meta?.tab || "home");

function go(tab) {
  if (route.path === tab.to) return;
  if (!isMainTabRoot(route) || route.name === "home") {
    router.push(tab.to);
    return;
  }
  router.replace(tab.to);
}
</script>

<template>
  <nav class="mobile-tabbar" :class="ui?.isDark ? 'mobile-tabbar-dark' : ''">
    <button
      v-for="tab in tabs"
      :key="tab.id"
      type="button"
      class="mobile-tabbar-item"
      data-press="off"
      :class="activeTab === tab.id ? 'is-active' : ''"
      @click="go(tab)"
    >
      <component :is="tab.icon" class="h-6 w-6" />
      <span>{{ tab.label }}</span>
    </button>
  </nav>
</template>
