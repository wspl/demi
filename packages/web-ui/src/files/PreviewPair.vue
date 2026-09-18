<script setup lang="ts">
/**
 * A change shown as its two versions side by side, each under its label: the
 * committed one and the working tree's, or a call's before and after. An
 * added file has only the second, a deleted file only the first; the columns
 * stack when the panel is narrow.
 */
withDefaults(defineProps<{ before?: string; after?: string }>(), {
  before: 'Committed',
  after: 'Working tree',
})
defineSlots<{
  before?: () => unknown
  after?: () => unknown
}>()
</script>

<template>
  <div class="grid h-full min-h-0 auto-rows-[minmax(0,1fr)] grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-px bg-line">
    <section v-if="$slots.before" class="flex min-h-0 flex-col bg-surface">
      <div class="flex h-7 shrink-0 items-center px-3 text-[11px] text-fg-muted">{{ before }}</div>
      <div class="min-h-0 flex-1">
        <slot name="before" />
      </div>
    </section>
    <section v-if="$slots.after" class="flex min-h-0 flex-col bg-surface">
      <div class="flex h-7 shrink-0 items-center px-3 text-[11px] text-fg-muted">{{ after }}</div>
      <div class="min-h-0 flex-1">
        <slot name="after" />
      </div>
    </section>
  </div>
</template>
