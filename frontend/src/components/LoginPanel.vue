<script setup>
defineProps({
  busyLogin: {
    type: Boolean,
    required: true,
  },
  loginForm: {
    type: Object,
    required: true,
  },
});

defineEmits(["submit"]);
</script>

<template>
  <div class="fixed inset-0 flex items-center justify-center">
    <div
      class="pointer-events-none absolute inset-0"
      style="background: radial-gradient(circle at top left, rgba(177, 77, 40, 0.18), transparent 28%), radial-gradient(circle at top right, rgba(29, 106, 97, 0.18), transparent 22%), linear-gradient(160deg, #f8f3ec 0%, #f4efe7 44%, #efe4d6 100%)"
    ></div>
    <div
      class="pointer-events-none absolute inset-0 opacity-25 [background-image:linear-gradient(rgba(125,98,73,0.06)_1px,transparent_1px),linear-gradient(90deg,rgba(125,98,73,0.06)_1px,transparent_1px)] [background-size:28px_28px] [mask-image:linear-gradient(to_bottom,rgba(0,0,0,0.75),transparent_88%)]"
    ></div>

    <section
      class="relative z-10 w-full max-w-md rounded-[2rem] border border-white/70 bg-white/70 p-8 shadow-[0_22px_60px_rgba(71,43,17,0.16)] backdrop-blur"
    >
      <div class="mb-6 text-center">
        <p class="mb-2 text-xs font-semibold uppercase tracking-[0.24em] text-amber-700">
          Rust + Axum + Vue
        </p>
        <h1
          class="text-3xl font-semibold text-stone-900"
          style="font-family: 'Space Grotesk', sans-serif"
        >
          Album
        </h1>
        <p class="mt-2 text-sm text-stone-500">登录到你的目录</p>
      </div>

      <form class="grid gap-4" @submit.prevent="$emit('submit')">
        <label class="grid gap-2">
          <span class="text-sm text-stone-600">用户名</span>
          <input
            v-model.trim="loginForm.username"
            autocomplete="username"
            class="rounded-2xl border border-stone-200 bg-white/85 px-4 py-3 text-stone-900 outline-none transition focus:border-amber-500 focus:ring-4 focus:ring-amber-100"
            placeholder="daniel / irene"
          />
        </label>

        <label class="grid gap-2">
          <span class="text-sm text-stone-600">密码</span>
          <input
            v-model.trim="loginForm.passphrase"
            autocomplete="current-password"
            class="rounded-2xl border border-stone-200 bg-white/85 px-4 py-3 text-stone-900 outline-none transition focus:border-amber-500 focus:ring-4 focus:ring-amber-100"
            placeholder="输入原始密码"
            type="password"
          />
        </label>

        <label class="flex items-center gap-3 text-sm text-stone-600">
          <input
            v-model="loginForm.keepLogin"
            class="h-4 w-4 rounded border-stone-300 text-amber-700 focus:ring-amber-200"
            type="checkbox"
          />
          <span>七天内保持登录</span>
        </label>

        <button
          class="inline-flex items-center justify-center rounded-2xl bg-stone-900 px-5 py-3 text-sm font-medium text-white transition hover:-translate-y-0.5 hover:bg-stone-800 disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:translate-y-0"
          :disabled="busyLogin"
          type="submit"
        >
          {{ busyLogin ? "登录中..." : "登录" }}
        </button>
      </form>
    </section>
  </div>
</template>