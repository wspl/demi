<script setup lang="ts">
import { ChevronDown } from '@lucide/vue'
import Button from './Button.vue'
import { ICON_PX } from './icon-metrics'

/**
 * `default` is a button; `ghost` a quiet inline chip; `field` a form control
 * that fills its row and shows a value (an icon and a name) the way an input
 * shows text.
 */
export type DropdownVariant = 'default' | 'ghost' | 'field'
export type DropdownSize = 'sm' | 'md'

withDefaults(defineProps<{
  isOpen: boolean
  variant?: DropdownVariant
  size?: DropdownSize
  ariaLabel?: string
  disabled?: boolean
}>(), {
  variant: 'default',
  size: 'md',
})
</script>

<template>
  <Button
    v-if="variant === 'default'"
    :aria-label="ariaLabel"
    :size="size"
    :pressed="isOpen"
    :disabled="disabled"
  >
    <slot />
    <ChevronDown
      :size="size === 'sm' ? ICON_PX.in24 : ICON_PX.in28"
      class="transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </Button>
  <span
    v-else-if="variant === 'field'"
    role="button"
    :aria-label="ariaLabel"
    class="flex w-full cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome text-fg transition-colors duration-200 ease-out"
    :class="[
      size === 'sm' ? 'h-6' : 'h-7',
      disabled
        ? 'pointer-events-none cursor-not-allowed bg-hover opacity-40'
        : isOpen ? 'bg-active' : 'bg-hover hover:bg-active',
    ]"
  >
    <slot />
    <ChevronDown
      :size="ICON_PX.in24"
      class="shrink-0 text-fg-subtle transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </span>
  <span
    v-else
    role="button"
    :aria-label="ariaLabel"
    class="inline-flex cursor-default select-none items-center gap-0.5 rounded-md text-chrome transition-colors duration-200 ease-out"
    :class="[
      size === 'sm' ? 'h-6 pl-1.5 pr-0.5 text-[12px]' : 'h-7 pl-2 pr-1',
      disabled
        ? 'pointer-events-none cursor-not-allowed opacity-40'
        : isOpen ? 'bg-hover text-fg-body' : 'text-fg-subtle hover:bg-hover hover:text-fg-muted',
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
