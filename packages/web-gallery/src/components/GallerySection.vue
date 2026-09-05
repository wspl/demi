<script setup lang="ts">
defineProps<{
  title: string
  note?: string
}>()
</script>

<template>
  <section class="space-y-3">
    <header class="space-y-1">
      <h2 class="text-[15px] font-medium text-fg-emphasis">{{ title }}</h2>
      <p v-if="note" class="max-w-3xl text-[13px] leading-5 text-fg-muted">{{ note }}</p>
    </header>
    <slot />
  </section>
</template>
