<script setup lang="ts" generic="T extends string">
import type { Component } from 'vue'
import { ICON_PX } from './icon-metrics'

/**
 * One of a few exclusive choices, each a card with an icon, a title and a line of
 * explanation, for a form that branches on the answer. A radio group, drawn the way
 * modern forms draw it: neutral cards on the surface, the chosen one outlined in the
 * accent with a faint accent wash and its icon in the accent; the text stays neutral.
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
      class="flex min-w-0 cursor-default select-none flex-col gap-1 rounded-xl border p-3 text-left outline-none transition-colors duration-200 ease-out focus-visible:ring-2 focus-visible:ring-line-focus"
      :class="model === option.value ? 'border-accent-fill bg-accent-fill/8' : 'border-line bg-surface-float hover:border-line-strong'"
      @click="model = option.value"
    >
      <span class="flex items-center gap-2 text-chrome font-medium text-fg-emphasis">
        <component :is="option.icon" :size="ICON_PX.in28" class="shrink-0" :class="model === option.value ? 'text-accent-fill' : 'text-fg-muted'" />
        <span class="choice-card-title truncate">{{ option.label }}</span>
      </span>
      <span class="choice-card-description text-[12px] leading-4 text-fg-muted">{{ option.description }}</span>
    </button>
  </div>
</template>
