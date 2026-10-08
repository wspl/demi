<script setup lang="ts">
import { ChevronDown } from '@lucide/vue'
import Button from './Button.vue'
import { pressOnKey } from './button-events'
import { ICON_PX } from './icon-metrics'

/**
 * `default` is a button; `ghost` a quiet inline chip; `field` a form control
 * that fills its row and shows a value (an icon and a name) the way an input
 * shows text. Each is as wide as what it holds unless it is made wider; then
 * the value stays at the start and the chevron moves to the end, so a
 * stretched button still reads as a picker, not as an action.
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
    <!-- The auto margin takes the width beyond the content, and only that. -->
    <ChevronDown
      :size="size === 'sm' ? ICON_PX.in24 : ICON_PX.in28"
      class="ml-auto transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </Button>
  <!-- Each face takes the focus as Button does (on its click, no Tab stop), so a closing menu
       can give it back, and Return or Space opens the menu again. -->
  <span
    v-else-if="variant === 'field'"
    role="button"
    :aria-label="ariaLabel"
    :tabindex="disabled ? undefined : -1"
    @keydown="pressOnKey"
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
    :tabindex="disabled ? undefined : -1"
    @keydown="pressOnKey"
    class="inline-flex cursor-default select-none items-center gap-0.5 rounded-md text-chrome transition-colors duration-200 ease-out"
    :class="[
      size === 'sm' ? 'h-6 pl-1.5 pr-0.5 text-[12px]' : 'h-7 pl-2 pr-1',
      disabled
        ? 'pointer-events-none cursor-not-allowed opacity-40'
        : isOpen ? 'bg-hover text-fg-body' : 'text-fg-subtle hover:bg-hover hover:text-fg-muted',
    ]"
  >
    <slot />
    <!-- The auto margin takes the width beyond the content, and only that. -->
    <ChevronDown
      :size="size === 'sm' ? ICON_PX.in24 : ICON_PX.in28"
      class="ml-auto transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </span>
</template>
