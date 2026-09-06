<script setup lang="ts">
/**
 * Label and explanation on the left, the control on the right. In a narrow card the
 * control drops under the text and keeps its right alignment.
 */
defineProps<{
  label: string
  description?: string
}>()
</script>

<template>
  <div class="flex min-h-14 flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 @sm:flex-nowrap">
    <div v-if="$slots.leading" class="flex shrink-0 self-start pt-[3px] text-fg-muted @sm:self-center @sm:pt-0">
      <slot name="leading" />
    </div>
    <div class="min-w-0 flex-1 select-none">
      <div class="text-chrome text-fg">{{ label }}</div>
      <div v-if="description" class="mt-0.5 text-[12px] leading-4 text-fg-subtle">{{ description }}</div>
    </div>
    <div v-if="$slots.default" class="flex basis-full items-center justify-end gap-2 @sm:basis-auto @sm:shrink-0 [&>*]:shrink-0">
      <slot />
    </div>
  </div>
</template>
