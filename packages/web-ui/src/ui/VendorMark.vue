<script setup lang="ts">
import { computed } from 'vue'

/**
 * A vendor's mark in the app's own ink: a monochrome SVG (models.dev logos, for one)
 * painted through a mask so it takes the current text color. Falls back to an initial.
 */
const props = withDefaults(defineProps<{
  label: string
  src?: string | null
  size?: 'sm' | 'md'
}>(), {
  src: null,
  size: 'md',
})

const initial = computed(() => props.label.trim().slice(0, 1).toUpperCase())
</script>

<template>
  <span
    class="inline-flex shrink-0 select-none items-center justify-center rounded-md bg-overlay/8 text-fg-muted"
    :class="size === 'sm' ? 'size-6' : 'size-7'"
    role="img"
    :aria-label="label"
  >
    <span
      v-if="src"
      class="block bg-current"
      :class="size === 'sm' ? 'size-3.5' : 'size-4'"
      :style="{ maskImage: `url(${src})`, maskSize: 'contain', maskRepeat: 'no-repeat', maskPosition: 'center' }"
    />
    <span v-else class="font-medium" :class="size === 'sm' ? 'text-[11px]' : 'text-[12px]'">{{ initial }}</span>
  </span>
</template>
