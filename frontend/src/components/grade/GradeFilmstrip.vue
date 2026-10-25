<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { VList } from "virtua/vue";
import { isVideoPhoto } from "../../utils/mediaKind.js";

const props = defineProps({
  photos: { type: Array, default: () => [] },
  currentPath: { type: String, default: "" },
  thumbFor: { type: Function, required: true },
  height: { type: Number, default: 96 },
});

const emit = defineEmits(["select", "load-more"]);

const END_PX = 500;
const thumb = computed(() => Math.max(40, props.height - 32));
const cell = computed(() => thumb.value + 4);

const hostRef = ref(null);
const listRef = ref(null);
let pending = false;
let resizeObserver = null;

function nearEnd() {
  const list = listRef.value;
  if (!list || !props.photos.length) return false;
  const viewport = list.viewportSize ?? 0;
  if (viewport < 8) return false;
  const leftover = (list.scrollSize ?? 0) - (list.scrollOffset ?? 0) - viewport;
  return leftover < END_PX;
}

function requestMore() {
  if (pending || !nearEnd()) return;
  pending = true;
  emit("load-more");
  window.setTimeout(() => { pending = false; }, 300);
}

function onScroll() {
  requestMore();
}

function onWheel(event) {
  if (Math.abs(event.deltaX) >= Math.abs(event.deltaY)) return;
  if (!event.deltaY) return;
  event.preventDefault();
  listRef.value?.scrollBy(event.deltaY);
}

function scrollCurrentIntoView() {
  const index = props.photos.findIndex((item) => item.path === props.currentPath);
  if (index < 0) return;
  listRef.value?.scrollToIndex(index, { align: "center" });
}

watch(() => props.currentPath, () => nextTick(scrollCurrentIntoView));
watch(() => props.photos.length, () => {
  nextTick(() => {
    pending = false;
    requestMore();
  });
});

onMounted(() => {
  resizeObserver = new ResizeObserver(() => requestMore());
  if (hostRef.value) resizeObserver.observe(hostRef.value);
  nextTick(requestMore);
});

onBeforeUnmount(() => {
  resizeObserver?.disconnect();
});

defineExpose({ scrollCurrentIntoView });
</script>

<template>
  <div ref="hostRef" class="app-split-t flex shrink-0 items-center px-2" :style="{ height: `${height}px` }" @wheel="onWheel">
    <VList
      ref="listRef"
      horizontal
      :data="photos"
      :item-size="cell"
      :buffer-size="800"
      class="min-w-0 flex-1 app-scrollbar"
      :style="{ height: `${thumb}px` }"
      @scroll="onScroll"
    >
      <template #default="{ item }">
        <div class="pr-1">
          <button type="button"
            class="overflow-hidden rounded border"
            :style="{ width: `${thumb}px`, height: `${thumb}px` }"
            :data-current="item.path === currentPath ? 'true' : 'false'"
            :disabled="isVideoPhoto(item)"
            :class="[
              item.path === currentPath ? 'border-amber-300' : 'border-transparent',
              isVideoPhoto(item) ? 'cursor-default opacity-30' : 'hover:border-stone-500',
            ]"
            @click="emit('select', item)">
            <img :src="thumbFor(item)" alt="" class="h-full w-full object-cover" />
          </button>
        </div>
      </template>
    </VList>
  </div>
</template>
