<script setup>
import { computed } from "vue";
import {
  faceAvatarRequestSize,
  faceCropStyle,
  faceThumbnailUrl,
  personCoverThumbnailUrl,
} from "../utils/faceCrop.js";

const props = defineProps({
  cover: { type: Object, default: null },
  /** Optional raw face box when cover is not a person cover object. */
  faceBox: { type: Array, default: null },
  path: { type: String, default: "" },
  owner: { type: String, default: "" },
  /** CSS display size of the avatar in pixels. */
  size: { type: Number, default: 128 },
  imgClass: { type: String, default: "" },
});

const dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1;
const requestSize = computed(() => faceAvatarRequestSize(props.size, dpr));

const box = computed(() => props.faceBox || props.cover?.face_box || null);

const usesFaceCropApi = computed(() => Boolean(
  box.value
  && (props.cover?.path || props.path),
));

const src = computed(() => {
  if (props.cover?.path) {
    return personCoverThumbnailUrl(props.cover, props.size);
  }
  if (!props.path) return "";
  if (usesFaceCropApi.value) {
    return faceThumbnailUrl(props.path, box.value, requestSize.value, props.owner);
  }
  const owner = props.owner ? `&owner=${encodeURIComponent(props.owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(props.path)}?size=${requestSize.value}${owner}`;
});

const cropStyle = computed(() => (usesFaceCropApi.value ? null : faceCropStyle(box.value)));
const imgClassFinal = computed(() => [
  props.imgClass,
  cropStyle.value ? "" : "h-full w-full object-cover",
]);
</script>

<template>
  <div class="relative h-full w-full overflow-hidden">
    <img
      v-if="src"
      :src="src"
      alt=""
      :class="imgClassFinal"
      :style="cropStyle || undefined"
      loading="lazy"
      decoding="async"
    />
  </div>
</template>
