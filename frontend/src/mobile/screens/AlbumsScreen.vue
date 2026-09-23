<script setup>
import { inject, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { t } from "../../i18n/index.js";
import { albumLocation } from "../router.js";
import { ALBUM_APP_KEY, MOBILE_UI_KEY } from "../context.js";
import Screen from "../layouts/Screen.vue";
import ActionSheet from "../sheets/ActionSheet.vue";
import { IconImage, IconPencil, IconPlus, IconRefresh, IconTrash } from "../../utils/icons";
import { albumCoverUrl } from "../../utils/albumCover.js";

defineOptions({ name: "AlbumsScreen" });

const app = inject(ALBUM_APP_KEY);
const ui = inject(MOBILE_UI_KEY);
const router = useRouter();

const menuAlbum = ref(null);
const pressTimer = ref(null);
const suppressClick = ref(false);

function coverUrl(album) {
  return albumCoverUrl(album, 720);
}

function openAlbum(album) {
  if (suppressClick.value) {
    suppressClick.value = false;
    return;
  }
  const dir = album.dir_name;
  app.selectAlbum(dir);
  router.push(albumLocation(dir));
}

function startPress(album) {
  cancelPress();
  pressTimer.value = window.setTimeout(() => {
    pressTimer.value = null;
    suppressClick.value = true;
    menuAlbum.value = album;
  }, 450);
}

function cancelPress() {
  if (pressTimer.value) {
    clearTimeout(pressTimer.value);
    pressTimer.value = null;
  }
}

function onLongPress(album, event) {
  event.preventDefault();
  menuAlbum.value = album;
}

function onAlbumAction(action) {
  const album = menuAlbum.value;
  menuAlbum.value = null;
  if (!album) return;
  const panel = ui.albumPanelRef;
  if (action.id === "share") panel?.openShareDialog(album);
  else if (action.id === "edit") panel?.openEditDialog(album);
  else if (action.id === "delete") panel?.confirmDeleteAlbum(album);
}

onMounted(() => {
  app.refreshAlbums();
});
</script>

<template>
  <Screen>
    <template #bar>
      <header class="mobile-topbar">
        <h1 class="min-w-0 flex-1 truncate font-display text-xl font-semibold">{{ t("nav.albums") }}</h1>
        <button type="button" class="mobile-icon-btn" :title="t('albums.refresh')" @click="app.refreshAlbums()">
          <IconRefresh class="h-5 w-5" />
        </button>
        <button type="button" class="mobile-icon-btn is-accent" :title="t('albums.create')" @click="ui.albumPanelRef?.openCreateDialog()">
          <IconPlus class="h-5 w-5" />
        </button>
      </header>
    </template>

    <div class="mobile-scroll px-4 pb-6">
      <div v-if="app.busy.albums && app.albums.length === 0" class="py-16 text-center text-sm opacity-60">{{ t("albums.loading") }}</div>
      <div v-else-if="app.albums.length === 0" class="flex flex-col items-center justify-center gap-3 py-20">
        <IconImage class="h-12 w-12 opacity-30" />
        <p class="text-sm opacity-60">{{ t("albums.empty") }}</p>
      </div>
      <div v-else class="grid grid-cols-1 gap-4 pt-3">
        <article
          v-for="album in app.albums"
          :key="album.dir_name"
          class="mobile-album-card"
          @click="openAlbum(album)"
          @pointerdown="startPress(album)"
          @pointerup="cancelPress"
          @pointermove="cancelPress"
          @pointercancel="cancelPress"
          @contextmenu.prevent="onLongPress(album, $event)"
        >
          <div class="relative aspect-[16/10] overflow-hidden bg-stone-200 dark:bg-stone-800">
            <img v-if="album.cover" :src="coverUrl(album)" alt="" class="h-full w-full object-cover" loading="lazy" />
            <div v-else class="flex h-full w-full items-center justify-center">
              <IconImage class="h-12 w-12 opacity-30" />
            </div>
            <span class="absolute bottom-2 right-2 rounded-full bg-black/55 px-2.5 py-0.5 text-[11px] text-white">
              {{ t("albums.photoCount", { count: album.photo_count }) }}
            </span>
          </div>
          <div class="px-3 py-3">
            <p class="truncate text-base font-semibold">{{ album.name || album.dir_name }}</p>
            <p v-if="album.description" class="mt-0.5 line-clamp-2 text-sm opacity-60">{{ album.description }}</p>
          </div>
        </article>
      </div>

      <template v-if="app.sharedAlbums.length > 0">
        <h3 class="mt-8 mb-3 text-xs font-semibold uppercase tracking-wide opacity-50">{{ t("albums.sharedAlbums") }}</h3>
        <div class="grid grid-cols-1 gap-4">
          <article
            v-for="album in app.sharedAlbums"
            :key="album.dir_name"
            class="mobile-album-card"
            @click="openAlbum(album)"
          >
            <div class="relative aspect-[16/10] overflow-hidden bg-stone-200">
              <img v-if="album.cover" :src="coverUrl(album)" alt="" class="h-full w-full object-cover" loading="lazy" />
              <span class="absolute top-2 left-2 rounded-full bg-blue-600/80 px-2 py-0.5 text-[10px] text-white">{{ album.owner }}</span>
              <span class="absolute bottom-2 right-2 rounded-full bg-black/55 px-2.5 py-0.5 text-[11px] text-white">
                {{ t("albums.photoCount", { count: album.photo_count }) }}
              </span>
            </div>
            <div class="px-3 py-3">
              <p class="truncate text-base font-semibold">{{ album.name || album.dir_name }}</p>
              <p class="mt-0.5 text-sm opacity-60">{{ t("albums.from", { owner: album.owner }) }}</p>
            </div>
          </article>
        </div>
      </template>
    </div>

    <ActionSheet
      :show="!!menuAlbum"
      :title="menuAlbum?.name || menuAlbum?.dir_name"
      :actions="[
        { id: 'share', label: t('albums.share'), icon: IconPencil },
        { id: 'edit', label: t('albums.edit'), icon: IconPencil },
        { id: 'delete', label: t('albums.deleteAlbum'), icon: IconTrash, danger: true },
      ]"
      @close="menuAlbum = null"
      @select="onAlbumAction"
    />
  </Screen>
</template>
