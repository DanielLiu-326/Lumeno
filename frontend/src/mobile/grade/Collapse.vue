<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

const props = defineProps({
  open: { type: Boolean, default: false },
  duration: { type: Number, default: 280 },
});

const body = ref(null);
let cleanup = null;
let seq = 0;

function finish(open) {
  const node = body.value;
  if (!node) return;
  node.style.height = open ? "auto" : "0px";
}

async function animate(open) {
  const node = body.value;
  if (!node) return;
  const my = ++seq;
  // The children are rendered by the same tick that flips `open`; measure after
  // the DOM update or the panel animates to a zero height.
  await nextTick();
  if (my !== seq || !body.value) return;
  if (cleanup) {
    node.removeEventListener("transitionend", cleanup);
    cleanup = null;
  }
  // Measure the real height, animate to/from it, then hand the height back to
  // the content so a later change (a new tool) is not clipped.
  const start = node.getBoundingClientRect().height;
  node.style.height = "auto";
  let end = open ? node.getBoundingClientRect().height : 0;
  if (open && end === 0) {
    // First render of a freshly selected tool: one more task so the content is
    // laid out before the target height is read.
    await new Promise((resolve) => setTimeout(resolve, 0));
    if (my !== seq || !body.value) return;
    end = node.getBoundingClientRect().height;
  }
  node.style.height = `${start}px`;
  node.style.overflow = "hidden";
  void node.offsetHeight;
  node.style.height = `${end}px`;
  cleanup = () => finish(open);
  node.addEventListener("transitionend", cleanup, { once: true });
}

watch(
  () => props.open,
  (open) => animate(open),
);

onMounted(() => {
  if (!body.value) return;
  body.value.style.transition = `height ${props.duration}ms cubic-bezier(0.22, 1, 0.36, 1)`;
  if (!props.open) body.value.style.height = "0px";
  else body.value.style.height = "auto";
});

onBeforeUnmount(() => {
  if (body.value && cleanup) body.value.removeEventListener("transitionend", cleanup);
});
</script>

<template>
  <div ref="body" class="overflow-hidden" data-collapse>
    <slot />
  </div>
</template>
