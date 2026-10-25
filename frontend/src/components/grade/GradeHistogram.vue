<script setup>
import { ref, watch } from "vue";

const props = defineProps({
  src: { type: String, default: "" },
  caption: { type: String, default: "" },
  isDark: { type: Boolean, default: false },
});

const canvas = ref(null);
let token = 0;

watch(() => props.src, (src) => {
  if (!src) {
    clear();
    return;
  }
  const current = ++token;
  const image = new Image();
  image.onload = () => {
    if (current !== token) return;
    draw(image);
  };
  image.src = src;
}, { immediate: true });

function clear() {
  const el = canvas.value;
  if (!el) return;
  el.getContext("2d").clearRect(0, 0, el.width, el.height);
}

function draw(image) {
  const el = canvas.value;
  if (!el) return;
  const width = el.clientWidth || 280;
  const height = el.clientHeight || 112;
  const ratio = window.devicePixelRatio || 1;
  el.width = Math.round(width * ratio);
  el.height = Math.round(height * ratio);
  const ctx = el.getContext("2d");
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  const sample = document.createElement("canvas");
  sample.width = 256;
  sample.height = 144;
  const sampleCtx = sample.getContext("2d", { willReadFrequently: true });
  sampleCtx.drawImage(image, 0, 0, sample.width, sample.height);
  const data = sampleCtx.getImageData(0, 0, sample.width, sample.height).data;
  const bins = [new Float32Array(256), new Float32Array(256), new Float32Array(256)];
  for (let i = 0; i < data.length; i += 4) {
    bins[0][data[i]] += 1;
    bins[1][data[i + 1]] += 1;
    bins[2][data[i + 2]] += 1;
  }
  for (const channel of bins) smooth(channel);
  let max = 1;
  for (const channel of bins) {
    for (const value of channel) if (value > max) max = value;
  }
  ctx.clearRect(0, 0, width, height);
  const colors = ["rgba(220,38,38,0.45)", "rgba(22,163,74,0.4)", "rgba(37,99,235,0.4)"];
  bins.forEach((channel, index) => {
    ctx.beginPath();
    ctx.moveTo(0, height);
    for (let x = 0; x < 256; x += 1) {
      const px = (x / 255) * width;
      const py = height - (channel[x] / max) * (height - 6);
      ctx.lineTo(px, py);
    }
    ctx.lineTo(width, height);
    ctx.closePath();
    ctx.fillStyle = colors[index];
    ctx.fill();
  });
}

function smooth(values) {
  const copy = Float32Array.from(values);
  for (let i = 0; i < values.length; i += 1) {
    let sum = 0;
    let count = 0;
    for (let k = -2; k <= 2; k += 1) {
      const j = i + k;
      if (j < 0 || j >= copy.length) continue;
      sum += copy[j];
      count += 1;
    }
    values[i] = sum / count;
  }
}
</script>

<template>
  <div class="app-split-b shrink-0">
    <canvas ref="canvas" class="block h-28 w-full"></canvas>
    <p class="truncate px-3 pb-2 text-center text-[11px]" :class="isDark ? 'text-stone-400' : 'text-stone-500'">{{ caption }}</p>
  </div>
</template>
