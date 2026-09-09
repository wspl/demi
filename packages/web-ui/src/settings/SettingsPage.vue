<script setup lang="ts">
/** One settings section: a title, an optional line under it, then groups. */
defineProps<{
  title: string
  description?: string
  /** Use the whole column: for a list beside its detail. */
  wide?: boolean
  /** Fill the host's height so a child can scroll on its own instead of the page. */
  fill?: boolean
}>()
</script>

<template>
  <div
    class="mx-auto flex w-full flex-col gap-8"
    :class="[wide ? 'max-w-none' : 'max-w-2xl', fill ? 'h-full min-h-0' : '']"
  >
    <header class="select-none">
      <h2 class="text-[20px] font-medium leading-7 text-fg-emphasis">{{ title }}</h2>
      <p v-if="description" class="mt-1 text-[13px] leading-5 text-fg-muted">{{ description }}</p>
    </header>
    <slot />
  </div>
</template>
