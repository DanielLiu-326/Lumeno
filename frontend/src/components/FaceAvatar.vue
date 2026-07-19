<script setup>
import { computed } from "vue";
import { faceCropStyle, personCoverThumbnailUrl } from "../utils/faceCrop.js";

const props = defineProps({
  cover: { type: Object, default: null },
  /** Optional raw face box when cover is not a person cover object. */
  faceBox: { type: Array, default: null },
  path: { type: String, default: "" },
  owner: { type: String, default: "" },
  size: { type: Number, default: 360 },
  imgClass: { type: String, default: "" },
});

const src = computed(() => {
  if (props.cover?.path) {
    return personCoverThumbnailUrl(props.cover, props.size);
  }
  if (!props.path) return "";
  const owner = props.owner ? `&owner=${encodeURIComponent(props.owner)}` : "";
  return `/api/thumbnail/${encodeURIComponent(props.path)}?size=${props.size}${owner}`;
});

const box = computed(() => props.faceBox || props.cover?.face_box || null);
const cropStyle = computed(() => faceCropStyle(box.value));
const isCropped = computed(() => cropStyle.value?.position === "absolute");
</script>

<template>
  <div class="relative h-full w-full overflow-hidden">
    <img
      v-if="src"
      :src="src"
      alt=""
      :class="[imgClass, isCropped ? '' : 'h-full w-full object-cover']"
      :style="cropStyle"
      loading="lazy"
      decoding="async"
    />
  </div>
</template>
