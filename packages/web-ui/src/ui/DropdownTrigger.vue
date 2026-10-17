<script setup lang="ts">
import type { Component } from 'vue'
import { ChevronDown } from '@lucide/vue'
import ChoiceIcon from './ChoiceIcon.vue'
import type { MenuIndicator } from './MenuItem.vue'
import Button from './Button.vue'
import InlineDetail from './InlineDetail.vue'
import type { InlineDetailText } from './inline-detail'
import { pressOnKey } from './button-events'
import { ICON_PX, useIconPx } from './icon-metrics'

/**
 * `default` is a button; `ghost` a quiet inline chip; `field` a form control
 * that fills its row and shows a value (an icon and a name) the way an input
 * shows text. Each is as wide as what it holds unless it is made wider; then
 * the value stays at the start and the chevron moves to the end, so a
 * stretched button still reads as a picker, not as an action.
 *
 * A `field` may carry a `detail` (InlineDetail): what its value stands
 * for, after it on the same line and baseline, smaller and in the
 * secondary color, such as what Automatic resolves to, `Automatic (Claude
 * Haiku 5.5)`. The field keeps its height; the detail truncates first, and
 * the value only once the detail is gone. Only a field shows a value, so
 * only a field draws a detail.
 *
 * Where the menu's items have icons of their own, any face shows the chosen
 * item's before its value, as its menu row draws it (ChoiceIcon): `icon`
 * with `indicator` as the row's dot on its corner, or the `icon` slot for an
 * icon drawn otherwise, such as a warning in its place. A Host's glyph
 * (`hostGlyph`) carries its state's dot itself. A choice with no icon of its
 * own stays text.
 */
export type DropdownVariant = 'default' | 'ghost' | 'field'
export type DropdownSize = 'sm' | 'md'

withDefaults(defineProps<{
  isOpen: boolean
  variant?: DropdownVariant
  size?: DropdownSize
  ariaLabel?: string
  disabled?: boolean
  /** What a `field`'s value stands for, after it in smaller secondary text. */
  detail?: InlineDetailText
  /** The chosen item's icon, the one its menu row shows. */
  icon?: Component
  /** The chosen item's status dot, on the icon's corner, as its menu row's. */
  indicator?: MenuIndicator
  indicatorLabel?: string
}>(), {
  variant: 'default',
  size: 'md',
})

const icons = useIconPx()
</script>

<template>
  <Button
    v-if="variant === 'default'"
    :aria-label="ariaLabel"
    :size="size"
    :pressed="isOpen"
    :disabled="disabled"
  >
    <ChoiceIcon v-if="icon || $slots.icon" :indicator="indicator" :indicator-label="indicatorLabel">
      <slot name="icon"><component :is="icon" :size="ICON_PX.in28" /></slot>
    </ChoiceIcon>
    <slot />
    <!-- The auto margin takes the width beyond the content, and only that. -->
    <ChevronDown
      :size="icons.chip"
      class="ml-auto transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </Button>
  <!-- Each face is a Tab stop and takes the focus as Button does, so a closing menu can give
       it back, and Return or Space opens the menu again. -->
  <span
    v-else-if="variant === 'field'"
    role="button"
    :aria-label="ariaLabel"
    :tabindex="disabled ? undefined : 0"
    @keydown="pressOnKey"
    @keyup="pressOnKey"
    class="flex w-full cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome text-fg transition-colors duration-200 ease-out"
    :class="[
      size === 'sm' ? 'h-6 phone:h-field' : 'h-field',
      disabled
        ? 'pointer-events-none cursor-not-allowed bg-hover opacity-40'
        : isOpen ? 'bg-active' : 'bg-hover hover:bg-active',
    ]"
  >
    <ChoiceIcon v-if="icon || $slots.icon" :indicator="indicator" :indicator-label="indicatorLabel">
      <slot name="icon"><component :is="icon" :size="ICON_PX.in28" /></slot>
    </ChoiceIcon>
    <InlineDetail v-if="detail" :detail="detail"><slot /></InlineDetail>
    <slot v-else />
    <ChevronDown
      :size="icons.chip"
      class="ml-auto shrink-0 text-fg-subtle transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </span>
  <span
    v-else
    role="button"
    :aria-label="ariaLabel"
    :tabindex="disabled ? undefined : 0"
    @keydown="pressOnKey"
    @keyup="pressOnKey"
    class="inline-flex cursor-default select-none items-center gap-0.5 rounded-md text-chrome transition-colors duration-200 ease-out"
    :class="[
      size === 'sm' ? 'h-6 pl-1.5 pr-0.5 text-control-sm phone:h-chip' : 'h-chip pl-2 pr-1',
      disabled
        ? 'pointer-events-none cursor-not-allowed opacity-40'
        : isOpen ? 'bg-hover text-fg-body' : 'text-fg-subtle hover:bg-hover hover:text-fg-muted',
    ]"
  >
    <ChoiceIcon v-if="icon || $slots.icon" class="mr-1" :indicator="indicator" :indicator-label="indicatorLabel">
      <slot name="icon"><component :is="icon" :size="ICON_PX.in28" /></slot>
    </ChoiceIcon>
    <slot />
    <!-- The auto margin takes the width beyond the content, and only that; a trigger that gives way
         cuts its content short, never the chevron. -->
    <ChevronDown
      :size="icons.chip"
      class="ml-auto shrink-0 transition-transform duration-200 ease-out"
      :class="isOpen ? 'rotate-180' : ''"
    />
  </span>
</template>
