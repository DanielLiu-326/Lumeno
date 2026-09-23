<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { IconFolder, IconAlbum, IconGear, IconCalendarDay, IconHome, IconSparkles, IconPower } from "../utils/icons";
import { t } from "../i18n/index.js";

const props = defineProps({
  isDark: {
    type: Boolean,
    default: false,
  },
  username: {
    type: String,
    required: true,
  },
  avatar: {
    type: String,
    default: null,
  },
  activeTab: {
    type: String,
    default: "dashboard",
  },
});

const emit = defineEmits(["switch-tab", "logout", "open-account"]);

const menuOpen = ref(false);
const avatarCircleRef = ref(null);
const cardStyle = ref({});
const avatarLetter = computed(() => (props.username ? props.username[0].toUpperCase() : "?"));
const avatarFailed = ref(false);
const showAvatarImg = computed(() => Boolean(props.avatar) && !avatarFailed.value);
watch(() => props.avatar, () => { avatarFailed.value = false; });

function placeCard() {
  const el = avatarCircleRef.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const width = 220;
  const left = rect.right + 4;
  const top = Math.max(8, Math.min(rect.top, window.innerHeight - 8));
  cardStyle.value = {
    position: "fixed",
    left: `${left}px`,
    top: `${top}px`,
    width: `${width}px`,
    zIndex: 80,
  };
}

async function toggleMenu() {
  menuOpen.value = !menuOpen.value;
  if (menuOpen.value) {
    await nextTick();
    placeCard();
  }
}

function openSettings() {
  menuOpen.value = false;
  emit("open-account");
}

function logout() {
  menuOpen.value = false;
  emit("logout");
}

function onDocumentClick(event) {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return;
  if (!target.closest("[data-user-menu]")) {
    menuOpen.value = false;
  }
}

function onDocumentKeydown(event) {
  if (event.key === "Escape") {
    menuOpen.value = false;
  }
}

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  document.addEventListener("keydown", onDocumentKeydown);
  window.addEventListener("resize", placeCard);
});
onBeforeUnmount(() => {
  document.removeEventListener("click", onDocumentClick);
  document.removeEventListener("keydown", onDocumentKeydown);
  window.removeEventListener("resize", placeCard);
});

const tabs = computed(() => [
  { key: "dashboard", label: t("nav.dashboard"), icon: IconHome },
  { key: "albums", label: t("nav.albums"), icon: IconAlbum },
  { key: "files", label: t("nav.files"), icon: IconFolder },
  { key: "on_this_day", label: t("nav.on_this_day"), icon: IconCalendarDay },
  { key: "daily_picks", label: t("nav.daily_picks"), icon: IconSparkles },
]);
</script>

<template>
  <aside class="app-frost-sidebar app-split-r relative z-10 flex h-full w-56 flex-col">
    <!-- Avatar / user section -->
    <div class="app-split-b px-4 py-6" data-user-menu>
      <button
        type="button"
        class="flex w-full flex-col items-center gap-3 rounded-2xl p-1 outline-none transition"
        :title="username"
        :aria-expanded="menuOpen"
        :aria-haspopup="true"
        @click.stop="toggleMenu"
      >
        <div
          ref="avatarCircleRef"
          class="h-14 w-14 overflow-hidden rounded-full ring-offset-2"
          :class="[
            showAvatarImg ? '' : 'flex items-center justify-center bg-gradient-to-br from-amber-500 to-orange-600 text-xl font-bold text-white',
            menuOpen
              ? (props.isDark ? 'ring-2 ring-amber-400' : 'ring-2 ring-amber-500')
              : 'hover:ring-2 hover:ring-amber-400/70',
          ]"
        >
          <img v-if="showAvatarImg" :src="`/api/avatar/${avatar}`" alt="" class="h-full w-full object-cover" @error="avatarFailed = true" />
          <span v-else>{{ avatarLetter }}</span>
        </div>
        <span class="text-sm font-medium" :class="props.isDark ? 'text-stone-100' : 'text-stone-800'">{{ username }}</span>
      </button>
    </div>

    <Teleport to="body">
      <Transition name="app-dropdown" :duration="{ enter: 300, leave: 200 }">
      <div
        v-if="menuOpen"
        data-user-menu
        class="app-dropdown-surface app-frost overflow-hidden rounded-2xl app-card p-2"
        :style="cardStyle"
        @click.stop
      >
        <div class="flex items-center gap-3 rounded-xl px-2 py-2.5">
          <div
            class="h-10 w-10 shrink-0 overflow-hidden rounded-full"
            :class="showAvatarImg ? '' : 'flex items-center justify-center bg-gradient-to-br from-amber-500 to-orange-600 text-sm font-bold text-white'"
          >
            <img v-if="showAvatarImg" :src="`/api/avatar/${avatar}`" alt="" class="h-full w-full object-cover" @error="avatarFailed = true" />
            <span v-else>{{ avatarLetter }}</span>
          </div>
          <p class="min-w-0 truncate text-sm font-semibold" :class="props.isDark ? 'text-stone-100' : 'text-stone-800'">
            {{ username }}
          </p>
        </div>
        <div class="my-1 h-px" :class="props.isDark ? 'bg-stone-700' : 'bg-stone-200'"></div>
        <button
          type="button"
          class="flex w-full items-center gap-2.5 rounded-xl px-3 py-2.5 text-left text-sm font-medium transition"
          :class="props.isDark ? 'text-stone-200 hover:bg-stone-800' : 'text-stone-700 hover:bg-stone-100'"
          @click="openSettings"
        >
          <IconGear class="h-4 w-4 shrink-0" />
          {{ t('nav.settings') }}
        </button>
        <button
          type="button"
          class="flex w-full items-center gap-2.5 rounded-xl px-3 py-2.5 text-left text-sm font-medium transition"
          :class="props.isDark ? 'text-rose-300 hover:bg-stone-800' : 'text-rose-700 hover:bg-rose-50'"
          @click="logout"
        >
          <IconPower class="h-4 w-4 shrink-0" />
          {{ t('topbar.logout') }}
        </button>
      </div>
      </Transition>
    </Teleport>

    <!-- Tabs -->
    <nav class="flex-1 px-3 py-4">
      <button
        v-for="tab in tabs"
        :key="tab.key"
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-sm font-medium transition"
        :class="activeTab === tab.key ? (props.isDark ? 'bg-amber-500/15 text-amber-200' : 'bg-amber-100/70 text-amber-900') : (props.isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-200 hover:text-stone-800')"
        @click="emit('switch-tab', tab.key)"
      >
        <component :is="tab.icon" class="h-5 w-5 shrink-0" />
        {{ tab.label }}
      </button>
    </nav>

    <!-- Settings button -->
    <div class="border-t px-3 py-3" :class="props.isDark ? 'border-stone-800' : 'border-stone-200/60'">
      <button
        class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-sm font-medium transition"
        :class="activeTab === 'settings' ? (props.isDark ? 'bg-amber-500/15 text-amber-200' : 'bg-amber-100/70 text-amber-900') : (props.isDark ? 'text-stone-300 hover:bg-stone-800 hover:text-stone-100' : 'text-stone-600 hover:bg-stone-200 hover:text-stone-800')"
        @click="emit('switch-tab', 'settings')"
      >
        <IconGear class="h-5 w-5 shrink-0" />
        {{ t('nav.settings') }}
      </button>
    </div>
  </aside>
</template>
