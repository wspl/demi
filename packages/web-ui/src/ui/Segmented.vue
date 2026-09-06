<script setup lang="ts" generic="T extends string">
import type { Component } from 'vue'
import { ICON_PX } from './icon-metrics'

/** One of a few exclusive choices, all visible. Same thumb as ToggleSwitch. */
export interface SegmentedOption<T extends string> {
  value: T
  label: string
  icon?: Component
}

withDefaults(defineProps<{
  options: readonly SegmentedOption<T>[]
  size?: 'sm' | 'md'
}>(), {
  size: 'md',
})

const model = defineModel<T>({ required: true })
</script>

<template>
  <div class="inline-flex w-fit shrink-0 items-stretch rounded-md bg-overlay/6 p-[2px]" role="radiogroup">
    <span
      v-for="option in options"
      :key="option.value"
      role="radio"
      :aria-checked="model === option.value"
      class="inline-flex cursor-default select-none items-center justify-center gap-1 whitespace-nowrap rounded-[5px] transition-[color,background-color,box-shadow] duration-200 ease-out"
      :class="[
        size === 'sm' ? 'px-1.5 py-0.5 text-[11px] leading-4' : 'h-6 px-2 text-[12px]',
        model === option.value ? 'segmented-thumb text-fg-emphasis' : 'text-fg-subtle hover:text-fg',
      ]"
      @click="model = option.value"
    >
      <component :is="option.icon" v-if="option.icon" :size="ICON_PX.in24" />
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
