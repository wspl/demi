<script setup lang="ts">
/** A titled card of rows. Rows separate themselves with hairlines. */
defineProps<{
  title?: string
  description?: string
}>()
</script>

<template>
  <section class="flex flex-col gap-3">
    <header v-if="title" class="select-none">
      <h3 class="text-[15px] font-medium leading-5 text-fg-emphasis">{{ title }}</h3>
      <p v-if="description" class="mt-0.5 text-[13px] leading-5 text-fg-muted">{{ description }}</p>
    </header>
    <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
      <slot />
    </div>
  </section>
</template>

<style>
.settings-card > * + * {
  border-top: 1px solid var(--line-subtle);
}
</style>
