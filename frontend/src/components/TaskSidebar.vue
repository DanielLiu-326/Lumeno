<script setup>
import { formatSize, progressPercent, statusLabel, statusTone } from "../utils/format";
import { IconClear } from "../utils/icons";
import { t } from "../i18n/index.js";

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

defineEmits(["clear-finished", "cancel-task", "pause-task", "resume-task"]);
</script>

<template>
  <aside
    class="min-w-0 rounded-[2rem] border border-white/70 bg-white/70 p-5 shadow-[0_22px_60px_rgba(71,43,17,0.16)] backdrop-blur md:p-6"
  >
    <div class="mb-4 flex items-start justify-between gap-4">
      <div>
        <p class="mb-2 text-xs font-semibold uppercase tracking-[0.24em] text-amber-700">{{ t("tasks.title") }}</p>
        <h2 class="font-display text-3xl font-semibold text-stone-900">{{ t("tasks.center") }}</h2>
      </div>
      <button
        v-if="loggedIn && tasks.length > 0"
        class="inline-flex h-10 w-10 items-center justify-center rounded-2xl bg-stone-200 text-stone-800 transition hover:-translate-y-0.5 hover:bg-stone-300"
        :title="t('tasks.clearFinished')"
        :aria-label="t('tasks.clearFinished')"
        @click="$emit('clear-finished')"
      >
        <IconClear class="h-4 w-4" />
      </button>
    </div>

    <section
      v-if="message.text"
      class="mb-4 rounded-[1.5rem] border-l-4 px-4 py-4"
      :class="message.kind === 'error' ? 'border-l-rose-600 bg-rose-50 text-rose-900' : 'border-l-emerald-600 bg-emerald-50 text-emerald-900'"
    >
      <strong class="block text-sm">{{ message.kind === "error" ? t("tasks.error") : t("tasks.info") }}</strong>
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
          <div class="flex shrink-0 items-center gap-2">
            <button
              v-if="task.pausable && task.status === 1"
              class="rounded-full bg-amber-100 px-2.5 py-1 text-[11px] font-medium text-amber-800 transition hover:bg-amber-200"
              @click="$emit('pause-task', task.id)"
            >{{ t("tasks.pause") }}</button>
            <button
              v-if="task.pausable && task.status === 2"
              class="rounded-full bg-emerald-100 px-2.5 py-1 text-[11px] font-medium text-emerald-800 transition hover:bg-emerald-200"
              @click="$emit('resume-task', task.id)"
            >{{ t("tasks.resume") }}</button>
            <button
              v-if="task.cancellable && (task.status === 0 || task.status === 1 || task.status === 2)"
              class="rounded-full bg-rose-100 px-2.5 py-1 text-[11px] font-medium text-rose-700 transition hover:bg-rose-200"
              @click="$emit('cancel-task', task.id)"
            >{{ t("tasks.cancel") }}</button>
            <span class="shrink-0 rounded-full px-2.5 py-1 text-xs font-medium" :class="statusTone(task.status)">
              {{ statusLabel(task.status) }}
            </span>
          </div>
        </div>

        <p v-if="task.phase_count" class="mb-1 text-[11px] font-medium uppercase tracking-wide text-amber-700">
          {{ task.phase_index }} / {{ task.phase_count }} · {{ task.phase_name }}
        </p>
        <div class="flex min-w-0 items-center justify-between gap-3">
          <p class="min-w-0 truncate text-sm text-stone-500" :title="task.current_task">{{ task.current_task || t("tasks.waitingForUpdate") }}</p>
          <span class="shrink-0 text-sm font-medium tabular-nums text-stone-600">{{ progressPercent(task) }}%</span>
        </div>

        <div class="mt-3 h-2.5 overflow-hidden rounded-full bg-stone-200">
          <div
            class="h-full rounded-full bg-linear-to-r from-amber-600 via-orange-400 to-emerald-600"
            :style="{ width: `${progressPercent(task)}%` }"
          ></div>
        </div>

        <div class="mt-3 flex flex-wrap gap-3 text-xs text-stone-500">
          <span>{{ task.processed_count }} / {{ task.total_count }} {{ t("tasks.items") }}</span>
          <span>{{ formatSize(task.processed_cost) }} / {{ formatSize(task.total_cost) }}</span>
        </div>

        <p v-if="task.error_msg" class="mt-3 text-sm text-rose-700">{{ task.error_msg }}</p>
      </div>

      <div v-if="tasks.length === 0" class="rounded-[1.5rem] border border-dashed border-stone-300 px-4 py-8 text-center text-sm text-stone-500">
        {{ t("tasks.noTasks") }}
      </div>
    </section>

    <section v-else class="rounded-[1.5rem] border border-dashed border-stone-300 px-4 py-5 text-sm text-stone-600">
      <h3 class="mb-3 font-medium text-stone-900">{{ t("tasks.connectedApis") }}</h3>
      <ul class="list-disc space-y-2 pl-5">
        <li>{{ t("tasks.apiLogin") }}</li>
        <li>{{ t("tasks.apiListFiles") }}</li>
        <li>{{ t("tasks.apiFileOps") }}</li>
        <li>{{ t("tasks.apiUpload") }}</li>
        <li>{{ t("tasks.apiTasks") }}</li>
      </ul>
    </section>
  </aside>
</template>