<script setup>
defineProps({
  show: { type: Boolean, required: true },
  zIndex: { type: [Number, String], default: 100 },
  centerClass: { type: String, default: "p-3 md:p-4" },
});

const emit = defineEmits(["close", "after-leave"]);
</script>

<template>
  <Teleport to="body">
    <Transition
      name="app-dialog"
      :duration="{ enter: 300, leave: 200 }"
      @after-leave="emit('after-leave')"
    >
      <div
        v-if="show"
        class="app-dialog-root"
        :style="{ zIndex }"
      >
        <div class="app-dialog-mask" @click="$emit('close')" />
        <div class="app-dialog-safe">
          <div class="app-dialog-stage" :class="centerClass">
            <slot />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
