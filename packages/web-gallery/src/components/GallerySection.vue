<script setup lang="ts">
import type { HeadlineText, SentenceText } from '@demicodes/web-ui/ui/ui-text'

defineProps<{
  title: HeadlineText
  note?: SentenceText
}>()
</script>

<template>
  <section :data-scroll-anchor="title" class="space-y-3">
    <header class="space-y-1">
      <h2 class="select-none text-[15px] font-medium text-fg-emphasis">{{ title }}</h2>
      <p v-if="note" class="max-w-3xl text-[13px] leading-5 text-fg-muted">{{ note }}</p>
    </header>
    <slot />
  </section>
</template>
