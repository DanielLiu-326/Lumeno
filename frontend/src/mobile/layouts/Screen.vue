<script setup>
defineProps({
  padded: { type: Boolean, default: false },
});
</script>

<template>
  <div class="mobile-screen flex h-full min-h-0 flex-1 flex-col overflow-hidden">
    <div class="mobile-screen-bar">
      <slot name="bar" />
    </div>
    <div
      class="mobile-screen-body flex min-h-0 flex-1 flex-col overflow-hidden"
      :class="padded ? 'px-4' : ''"
    >
      <slot />
    </div>
  </div>
</template>
