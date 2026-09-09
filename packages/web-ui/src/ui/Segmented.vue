<script setup lang="ts" generic="T extends string">
import { computed } from 'vue'
import type { Component } from 'vue'
import { ICON_PX } from './icon-metrics'

/**
 * One of a few exclusive choices, all visible. Segments share one width so the
 * thumb is a single element that slides to the chosen one instead of re-appearing.
 */
export interface SegmentedOption<T extends string> {
  value: T
  label: string
  icon?: Component
}

const props = withDefaults(defineProps<{
  options: readonly SegmentedOption<T>[]
  size?: 'sm' | 'md'
}>(), {
  size: 'md',
})

const model = defineModel<T>({ required: true })

const selectedIndex = computed(
  () =>
    Math.max(0, props.options.findIndex((option) => option.value === model.value))
)
</script>

<template>
  <div
    class="relative grid w-fit shrink-0 auto-cols-fr grid-flow-col rounded-md bg-overlay/6 p-[2px]"
    role="radiogroup"
  >
    <span
      aria-hidden="true"
      class="segmented-thumb pointer-events-none absolute inset-y-[2px] left-[2px] rounded-[5px] transition-transform duration-200 ease-out motion-reduce:transition-none"
      :style="{
        width: `calc((100% - 4px) / ${options.length})`,
        transform: `translateX(${selectedIndex * 100}%)`,
      }"
    />
    <span
      v-for="option in options"
      :key="option.value"
      role="radio"
      :aria-checked="model === option.value"
      class="relative z-10 inline-flex cursor-default select-none items-center justify-center gap-1 whitespace-nowrap rounded-[5px] transition-colors duration-200 ease-out"
      :class="[
        size === 'sm' ? 'px-1.5 py-0.5 text-[11px] leading-4' : 'h-6 px-2 text-[12px]',
        model === option.value ? 'text-fg-emphasis' : 'text-fg-subtle hover:text-fg',
      ]"
      @click="model = option.value"
    >
      <component
        :is="option.icon"
        v-if="option.icon"
        :size="ICON_PX.in24"
      />
      {{ option.label }}
    </span>
  </div>
</template>

<style scoped>
.segmented-thumb {
  background: var(--btn-bg);
  box-shadow: var(--shadow-btn);
}

html[data-theme="dark"] .segmented-thumb {
  background: color-mix(in srgb, var(--color-overlay) 12%, var(--color-surface));
}
</style>
