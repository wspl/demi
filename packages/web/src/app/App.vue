<script setup lang="ts">
import { ref } from 'vue'

const theme = ref<'dark' | 'light'>('dark')

function toggleTheme() {
  theme.value = theme.value === 'dark' ? 'light' : 'dark'
  document.documentElement.setAttribute('data-theme', theme.value)
}

const fgTokens = ['text-fg-emphasis', 'text-fg', 'text-fg-body', 'text-fg-muted', 'text-fg-subtle', 'text-fg-faint', 'text-fg-ghost']
const surfaceTokens = ['bg-surface-base', 'bg-surface', 'bg-surface-editor', 'bg-surface-raised']
const tintTokens = ['text-on-danger', 'text-on-success', 'text-on-warning', 'text-on-accent', 'text-on-violet']
</script>

<template>
  <div class="flex h-full flex-col gap-6 overflow-auto bg-surface-base p-8 text-fg">
    <header class="flex items-center justify-between">
      <h1 class="text-lg font-semibold text-fg-emphasis">Demi Web — design tokens</h1>
      <span
        class="cursor-pointer rounded-lg bg-surface-raised px-3 py-1.5 text-sm text-fg-muted shadow-md transition-colors hover:bg-active"
        @click="toggleTheme"
      >
        theme: {{ theme }}
      </span>
    </header>

    <section class="rounded-xl bg-surface p-5 shadow-lg">
      <h2 class="mb-3 text-sm font-medium text-fg-muted">Foreground scale</h2>
      <p v-for="token in fgTokens" :key="token" :class="token" class="text-sm leading-relaxed">
        {{ token }} — The quick brown fox jumps over the lazy dog.
      </p>
    </section>

    <section class="rounded-xl bg-surface p-5 shadow-lg">
      <h2 class="mb-3 text-sm font-medium text-fg-muted">Surfaces</h2>
      <div class="flex flex-wrap gap-3">
        <div
          v-for="token in surfaceTokens"
          :key="token"
          :class="token"
          class="flex h-20 w-40 items-center justify-center rounded-lg border border-line text-xs text-fg-subtle"
        >
          {{ token }}
        </div>
      </div>
    </section>

    <section class="rounded-xl bg-surface p-5 shadow-lg">
      <h2 class="mb-3 text-sm font-medium text-fg-muted">Accent tints</h2>
      <div class="flex flex-wrap gap-4">
        <span v-for="token in tintTokens" :key="token" :class="token" class="text-sm font-medium">{{ token }}</span>
      </div>
    </section>
  </div>
</template>
