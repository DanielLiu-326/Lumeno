<script setup>
import { IconStar, IconStarOff } from "../../utils/icons";

const props = defineProps({
  modelValue: { type: Object, required: true },
  isDark:     { type: Boolean, default: false },
  avail:      { type: Object, default: () => ({}) },
});
const emit = defineEmits(["update:modelValue", "apply"]);

function toggle(star) {
  const ratings = [...(props.modelValue.ratings || [])];
  const idx = ratings.indexOf(star);
  if (idx >= 0) ratings.splice(idx, 1);
  else { ratings.push(star); ratings.sort((a, b) => a - b); }
  emit("update:modelValue", { ...props.modelValue, ratings });
  emit("apply");
}
</script>

<template>
  <div class="flex items-center gap-0.5">
    <button
      v-for="star in [0, 1, 2, 3, 4, 5]"
      :key="star"
      class="p-0.5 transition-colors"
      :class="(modelValue.ratings || []).includes(star)
        ? 'text-amber-400'
        : (isDark ? 'text-stone-600 hover:text-stone-400' : 'text-stone-300 hover:text-stone-500')"
      @click="toggle(star)"
    >
      <IconStarOff v-if="star === 0" class="h-5 w-5" />
      <IconStar v-else class="h-5 w-5" />
    </button>
  </div>
</template>
