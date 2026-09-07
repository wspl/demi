<script setup lang="ts" generic="T extends string">
import type { Component } from 'vue'
import { ICON_PX } from './icon-metrics'

/**
 * One of a few exclusive choices, each a card with an icon, a title and a line of
 * explanation, for a form that branches on the answer. A radio group; the chosen card
 * wears the accent ring.
 */
export interface ChoiceCardOption<T extends string> {
  value: T
  label: string
  description: string
  icon: Component
}

defineProps<{
  options: readonly ChoiceCardOption<T>[]
}>()

const model = defineModel<T>({ required: true })
</script>

<template>
  <div class="grid auto-cols-fr grid-flow-col gap-2" role="radiogroup">
    <button
      v-for="option in options"
      :key="option.value"
      type="button"
      role="radio"
      :aria-checked="model === option.value"
      class="flex min-w-0 cursor-default select-none flex-col gap-1.5 rounded-xl border p-3 text-left transition-colors duration-200 ease-out"
      :class="model === option.value ? 'border-line-focus bg-tint-accent' : 'border-line bg-surface-float hover:bg-hover'"
      @click="model = option.value"
    >
      <span class="flex items-center gap-2 text-chrome" :class="model === option.value ? 'text-fg-emphasis' : 'text-fg'">
        <component :is="option.icon" :size="ICON_PX.in28" class="shrink-0" :class="model === option.value ? 'text-fg-emphasis' : 'text-fg-muted'" />
        <span class="truncate font-medium">{{ option.label }}</span>
      </span>
      <span class="text-[12px] leading-4 text-fg-subtle">{{ option.description }}</span>
    </button>
  </div>
</template>
