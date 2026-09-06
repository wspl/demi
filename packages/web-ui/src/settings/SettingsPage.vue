<script setup lang="ts">
/** One settings section: a title, an optional line under it, then groups. */
defineProps<{
  title: string
  description?: string
}>()
</script>

<template>
  <div class="mx-auto flex max-w-2xl flex-col gap-8">
    <header class="select-none">
      <h2 class="text-[20px] font-medium leading-7 text-fg-emphasis">{{ title }}</h2>
      <p v-if="description" class="mt-1 text-[13px] leading-5 text-fg-muted">{{ description }}</p>
    </header>
    <slot />
  </div>
</template>
