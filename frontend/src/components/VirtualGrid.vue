<script setup>
/**
 * VirtualGrid — 虚拟化网格组件
 *
 * 通过 ResizeObserver + getComputedStyle 探测实际列数（支持 Tailwind 响应式 grid-cols-*），
 * 将列表按行分组后交给 virtua 的 VList 虚拟化渲染，避免大量条目同时存在于 DOM 中。
 *
 * Props:
 *   items        — 数据数组
 *   itemKey      — 唯一键字段名，或返回 key 的函数
 *   gridClass    — CSS grid 类名（含 grid-cols-* 和 gap-*），同时用作探测元素
 *   overscan     — 视口外多渲染的行数（默认 2）
 *   bottomThreshold — 距底部多少 px 时触发 scroll-near-bottom（默认 500）
 *
 * Emits:
 *   scroll-near-bottom — 滚动接近底部时触发（已内置去抖）
 *
 * Slots:
 *   default: { item, index } — 渲染单个格子
 */

import { ref, computed, onMounted, onBeforeUnmount, watch, nextTick } from "vue";
import { VList } from "virtua/vue";

const props = defineProps({
  items: { type: Array, required: true },
  itemKey: { type: [String, Function], required: true },
  gridClass: { type: String, required: true },
  overscan: { type: Number, default: 2 },
  bottomThreshold: { type: Number, default: 500 },
});

const emit = defineEmits(["scroll-near-bottom"]);

const listRef = ref(null);
const probeRef = ref(null);
const colCount = ref(1);
const rowGapPx = ref(0);

let ro = null;
let nearBottomPending = false;

/** 从探测元素的 computedStyle 读取列数和行间距 */
function measureGrid() {
  if (!probeRef.value) return;
  const style = window.getComputedStyle(probeRef.value);

  const colsStr = style.gridTemplateColumns;
  if (colsStr && colsStr !== "none") {
    const count = colsStr.trim().split(/\s+/).filter(Boolean).length;
    if (count > 0 && count !== colCount.value) {
      colCount.value = count;
    }
  }

  const gap = parseFloat(style.rowGap);
  rowGapPx.value = isNaN(gap) ? 0 : gap;
}

onMounted(() => {
  measureGrid();
  ro = new ResizeObserver(measureGrid);
  if (probeRef.value) ro.observe(probeRef.value);
});

onBeforeUnmount(() => {
  ro?.disconnect();
  ro = null;
});

// gridClass 变化时（如用户调整缩略图大小），重新探测
watch(() => props.gridClass, () => nextTick(measureGrid));

/**
 * 将扁平的 items 按 colCount 分组成行。
 * 每行记录起始索引方便 slot 暴露全局 index。
 */
const rows = computed(() => {
  const n = Math.max(1, colCount.value);
  const result = [];
  for (let i = 0; i < props.items.length; i += n) {
    result.push({ startIndex: i, items: props.items.slice(i, i + n) });
  }
  return result;
});

function getKey(item) {
  if (typeof props.itemKey === "function") return props.itemKey(item);
  return item[props.itemKey];
}

function onScroll(offset) {
  if (nearBottomPending) return;
  const vl = listRef.value;
  if (!vl) return;
  if (vl.scrollSize - offset - vl.viewportSize < props.bottomThreshold) {
    nearBottomPending = true;
    setTimeout(() => { nearBottomPending = false; }, 300);
    emit("scroll-near-bottom");
  }
}
</script>

<template>
  <div class="relative h-full">
    <!--
      探测元素：与主网格使用相同的 CSS 类，
      通过 getComputedStyle 读取实际列数与行间距。
      visibility:hidden 保留布局但不显示；absolute 不占文档流。
    -->
    <div
      ref="probeRef"
      :class="gridClass"
      class="pointer-events-none invisible absolute inset-x-0"
      aria-hidden="true"
    >
      <div v-for="i in 12" :key="i" />
    </div>

    <!--
      VList 内部虚拟化各行，支持可变行高（virtua 会自动测量）。
      key 绑定 colCount 确保列数变化时完整重建，避免高度缓存混乱。
      重要：VList 必须使用 :data 传入数组 + #default 作用域插槽，
      不能在内部写 v-for。
    -->
    <VList
      ref="listRef"
      :key="colCount"
      :data="rows"
      class="app-scrollbar absolute inset-0"
      :bufferSize="400"
      @scroll="onScroll"
    >
      <template #default="{ item: row }">
        <div
          :class="gridClass"
          :style="rowGapPx > 0 ? { paddingBottom: rowGapPx + 'px' } : undefined"
        >
          <slot
            v-for="(item, colIdx) in row.items"
            :key="getKey(item)"
            :item="item"
            :index="row.startIndex + colIdx"
          />
        </div>
      </template>
    </VList>
  </div>
</template>
