<script setup>
import { computed, onBeforeUnmount, ref } from "vue";
import FaceAvatar from "../../components/FaceAvatar.vue";
import LazyImage from "../../components/LazyImage.vue";
import VirtualGrid from "../../components/VirtualGrid.vue";
import { t } from "../../i18n/index.js";
import { IconCheck, IconImage, IconStarOff } from "../../utils/icons";
import ActionSheet from "../sheets/ActionSheet.vue";
import { albumPhotoPath, detailItemBox, detailItemKey, isCurrentCover, photoThumb } from "./faceModel.js";

const props = defineProps({
  items: { type: Array, default: () => [] },
  isDark: { type: Boolean, default: false },
  albumName: { type: String, default: "" },
  albumOwner: { type: String, default: "" },
  person: { type: Object, default: null },
  editable: { type: Boolean, default: false },
  showOriginal: { type: Boolean, default: false },
});

const emit = defineEmits(["open", "set-cover", "clear-cover"]);

const menuItem = ref(null);
let press = null;
let timer = 0;
let swallowClick = false;

const menuActions = computed(() => {
  const item = menuItem.value;
  if (!item || !props.person) return [];
  const current = isCurrentCover(props.person, item, props.albumName);
  return [{
    id: current ? "clear" : "set",
    label: t(current ? "people.clearCover" : "people.setCover"),
    icon: current ? IconStarOff : IconImage,
  }];
});

function clearPress() {
  window.removeEventListener("pointermove", onMove);
  window.removeEventListener("pointerup", onUp);
  window.removeEventListener("pointercancel", onCancel);
  press = null;
}

function cancelTimer() {
  if (!timer) return;
  clearTimeout(timer);
  timer = 0;
}

function onDown(item, event) {
  if (!props.editable || !props.person) return;
  if (event.button != null && event.button !== 0) return;
  clearPress();
  cancelTimer();
  press = { item, x: event.clientX, y: event.clientY, moved: false };
  timer = window.setTimeout(() => {
    timer = 0;
    if (!press || press.moved) return;
    menuItem.value = press.item;
    swallowClick = true;
    window.setTimeout(() => {
      swallowClick = false;
    }, 700);
  }, 450);
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
  window.addEventListener("pointercancel", onCancel);
}

function onMove(event) {
  if (!press || press.moved) return;
  if (Math.abs(event.clientX - press.x) > 12 || Math.abs(event.clientY - press.y) > 12) {
    press.moved = true;
    cancelTimer();
  }
}

function onUp() {
  clearPress();
  cancelTimer();
}

function onCancel() {
  clearPress();
  cancelTimer();
}

function onClick(item) {
  if (swallowClick) {
    swallowClick = false;
    return;
  }
  emit("open", item);
}

function onCoverAction(action) {
  const item = menuItem.value;
  menuItem.value = null;
  if (!item || !action?.id) return;
  if (action.id === "clear") emit("clear-cover");
  else emit("set-cover", item);
}

onBeforeUnmount(() => {
  clearPress();
  cancelTimer();
});
</script>

<template>
  <div class="relative min-h-0 flex-1 overflow-hidden">
    <VirtualGrid
      :items="items"
      :item-key="detailItemKey"
      grid-class="grid grid-cols-3 gap-1.5 px-3 py-3"
      class="absolute inset-0"
    >
      <template #default="{ item }">
        <article
          class="relative cursor-pointer overflow-hidden rounded-xl border"
          :class="person && isCurrentCover(person, item, albumName)
            ? (isDark ? 'border-amber-500 ring-1 ring-amber-500/60' : 'border-amber-500 ring-1 ring-amber-400/70')
            : (isDark ? 'border-stone-700' : 'border-stone-200')"
          @pointerdown="onDown(item, $event)"
          @click="onClick(item)"
          @contextmenu.prevent
        >
          <div class="aspect-square" :class="isDark ? 'bg-stone-800' : 'bg-stone-100'">
            <LazyImage
              v-if="showOriginal"
              :src="photoThumb(albumPhotoPath(albumName, item.source), albumOwner)"
              img-class="h-full w-full object-cover"
              alt=""
            />
            <FaceAvatar
              v-else-if="item.source && detailItemBox(item)"
              :album="albumName"
              :face-id="item.face_id || ''"
              :source="item.source || ''"
              :face-box="detailItemBox(item)"
              :size="160"
            />
          </div>
          <span
            v-if="person && isCurrentCover(person, item, albumName)"
            class="absolute left-1 top-1 inline-flex items-center gap-0.5 rounded-md bg-amber-600/90 px-1 py-0.5 text-[9px] font-medium text-white"
          >
            <IconCheck class="h-2.5 w-2.5" />{{ t("people.currentCover") }}
          </span>
        </article>
      </template>
    </VirtualGrid>
    <ActionSheet
      :show="!!menuItem"
      :actions="menuActions"
      @close="menuItem = null"
      @select="onCoverAction"
    />
  </div>
</template>
