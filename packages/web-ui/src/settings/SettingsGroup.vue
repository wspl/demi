<script setup lang="ts">
/**
 * A titled card of rows. Rows separate themselves with hairlines. A `header` slot
 * replaces the title when the group's subject needs its own controls, such as an
 * editable name beside its actions; an `aside` slot puts something beside the card,
 * such as a live preview of what the rows change.
 */
defineProps<{
  title?: string
  description?: string
}>()
</script>

<template>
  <section class="flex flex-col gap-3">
    <slot name="header">
      <header v-if="title" class="select-none">
        <h3 class="text-[15px] font-medium leading-5 text-fg-emphasis">{{ title }}</h3>
        <p v-if="description" class="mt-0.5 text-[13px] leading-5 text-fg-muted">{{ description }}</p>
      </header>
    </slot>
    <!-- An aside (a preview, a legend) sits beside the card where there is room, under it where not. -->
    <div class="@container flex flex-col gap-4 @lg:flex-row @lg:items-stretch">
      <div class="settings-card @container min-w-0 flex-1 overflow-hidden rounded-xl border border-line bg-surface-float">
        <slot />
      </div>
      <div v-if="$slots.aside" class="flex shrink-0">
        <slot name="aside" />
      </div>
    </div>
  </section>
</template>

<style>
.settings-card > * + * {
  border-top: 1px solid var(--line-subtle);
}
</style>
