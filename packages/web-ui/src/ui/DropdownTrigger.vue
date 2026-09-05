<script setup lang="ts">
import { ChevronDown } from '@lucide/vue'
import Button from './Button.vue'
import { ICON_PX } from './icon-metrics'

export type DropdownVariant = 'default' | 'ghost'
export type DropdownSize = 'sm' | 'md'

withDefaults(defineProps<{
  isOpen: boolean
  variant?: DropdownVariant
  size?: DropdownSize
}>(), {
  variant: 'default',
  size: 'md',
})
</script>

<template>
  <Button
    v-if="variant === 'default'"
    :size="size"
    :pressed="isOpen"
  >
    <slot />
    <ChevronDown
      :size="size === 'sm' ? ICON_PX.in24 : ICON_PX.in28"
      class="transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </Button>
  <span
    v-else
    role="button"
    class="inline-flex cursor-default select-none items-center gap-0.5 rounded-md text-chrome transition-colors duration-200 ease-out"
    :class="[
      size === 'sm' ? 'h-6 pl-1.5 pr-0.5 text-[12px]' : 'h-7 pl-2 pr-1',
      isOpen ? 'bg-hover text-fg-body' : 'text-fg-subtle hover:bg-hover hover:text-fg-muted',
    ]"
  >
    <slot />
    <ChevronDown
      :size="size === 'sm' ? ICON_PX.in24 : ICON_PX.in28"
      class="transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </span>
</template>
