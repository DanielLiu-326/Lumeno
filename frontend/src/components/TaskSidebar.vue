<script setup>
import { formatSize, progressPercent, statusLabel, statusTone } from "../utils/format";
import { IconCheck } from "../utils/icons";

defineProps({
  loggedIn: {
    type: Boolean,
    required: true,
  },
  message: {
    type: Object,
    required: true,
  },
  tasks: {
    type: Array,
    required: true,
  },
});

defineEmits(["clear-finished"]);
</script>

<template>
  <aside
    class="min-w-0 rounded-[2rem] border border-white/70 bg-white/70 p-5 shadow-[0_22px_60px_rgba(71,43,17,0.16)] backdrop-blur md:p-6"
  >
    <div class="mb-4 flex items-start justify-between gap-4">
      <div>
        <p class="mb-2 text-xs font-semibold uppercase tracking-[0.24em] text-amber-700">后台任务</p>
        <h2 class="font-display text-3xl font-semibold text-stone-900">任务中心</h2>
      </div>
      <button
        v-if="loggedIn && tasks.length > 0"
        class="inline-flex items-center gap-1.5 rounded-2xl bg-stone-200 px-4 py-2.5 text-sm font-medium text-stone-800 transition hover:-translate-y-0.5 hover:bg-stone-300"
        @click="$emit('clear-finished')"
      >
        <IconCheck class="h-4 w-4" />
        清理已完成
      </button>
    </div>

    <section
      v-if="message.text"
      class="mb-4 rounded-[1.5rem] border-l-4 px-4 py-4"
      :class="message.kind === 'error' ? 'border-l-rose-600 bg-rose-50 text-rose-900' : 'border-l-emerald-600 bg-emerald-50 text-emerald-900'"
    >
      <strong class="block text-sm">{{ message.kind === "error" ? "错误" : "提示" }}</strong>
      <p class="mt-2 text-sm leading-6">{{ message.text }}</p>
    </section>

    <section v-if="loggedIn" class="grid gap-3">
      <div
        v-for="task in tasks"
        :key="task.id"
        class="min-w-0 overflow-hidden rounded-[1.5rem] border border-white/70 bg-white/75 p-4 shadow-sm"
      >
        <div class="mb-2 flex items-start justify-between gap-3">
          <strong class="min-w-0 truncate text-sm text-stone-900" :title="task.name">{{ task.name }}</strong>
          <span class="shrink-0 rounded-full px-2.5 py-1 text-xs font-medium" :class="statusTone(task.status)">
            {{ statusLabel(task.status) }}
          </span>
        </div>

        <p class="truncate text-sm text-stone-500" :title="task.current_task">{{ task.current_task || "等待任务更新" }}</p>

        <div class="mt-3 h-2.5 overflow-hidden rounded-full bg-stone-200">
          <div
            class="h-full rounded-full bg-linear-to-r from-amber-600 via-orange-400 to-emerald-600"
            :style="{ width: `${progressPercent(task)}%` }"
          ></div>
        </div>

        <div class="mt-3 flex flex-wrap gap-3 text-xs text-stone-500">
          <span>{{ task.processed_count }} / {{ task.total_count }} 项</span>
          <span>{{ formatSize(task.processed_cost) }} / {{ formatSize(task.total_cost) }}</span>
        </div>

        <p v-if="task.error_msg" class="mt-3 text-sm text-rose-700">{{ task.error_msg }}</p>
      </div>

      <div v-if="tasks.length === 0" class="rounded-[1.5rem] border border-dashed border-stone-300 px-4 py-8 text-center text-sm text-stone-500">
        暂无后台任务。
      </div>
    </section>

    <section v-else class="rounded-[1.5rem] border border-dashed border-stone-300 px-4 py-5 text-sm text-stone-600">
      <h3 class="mb-3 font-medium text-stone-900">前端已接入的接口</h3>
      <ul class="list-disc space-y-2 pl-5">
        <li>登录 /api/login</li>
        <li>目录浏览 /api/list_files</li>
        <li>下载、删除、重命名、新建文件夹</li>
        <li>上传 /api/upload + /api/upload_feed</li>
        <li>任务列表 /api/get_task_list/</li>
      </ul>
    </section>
  </aside>
</template>