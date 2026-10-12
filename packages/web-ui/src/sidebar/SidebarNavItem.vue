<script setup lang="ts">
import { computed } from 'vue'
import type { Component } from 'vue'
import { disabledTooltip } from '@demicodes/web-ui/ui/disabled'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { usePhone } from '@demicodes/web-ui/ui/phone'
import { pressOnKey } from '@demicodes/web-ui/ui/button-events'
import type { SentenceText, TitleText } from '../ui/ui-text'

/**
 * One entry outside the list: a primary action or a settings section it opens, or a place in
 * a rail.
 */
const props = defineProps<{
  icon?: Component
  label: TitleText
  shortcut?: string
  pressed?: boolean
  /** lg is a list that is a page of its own, as narrow settings are; on a phone either is a finger's height. */
  size?: 'md' | 'lg'

  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
  /** What the entry stands for, as a tooltip, such as a place's full path; a disabled entry's reason comes first. */
  hint?: SentenceText
}>()

const phone = usePhone()
const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason) ?? props.hint)

const emit = defineEmits<{
  click: []
}>()
</script>

<template>
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="div"
    :open-delay-ms="80"
  >
    <div
      role="button"
      :aria-label="label"
      :aria-disabled="disabled || undefined"
      :tabindex="disabled ? undefined : 0"
      class="focus-ring flex cursor-default select-none items-center gap-2 rounded-md transition-colors duration-200 ease-out"
      :class="[
        'pl-2.5 pr-2',
        size === 'lg' ? 'h-10 text-[15px] phone:h-11 phone:text-[17px]' : 'h-7 text-chrome phone:h-11 phone:text-[16px]',
        'phone:gap-3 phone:pl-3',
        disabled
          ? 'cursor-not-allowed text-fg-faint'
          : pressed
            ? 'bg-active text-fg-emphasis'
            : 'text-fg-muted hover:bg-hover hover:text-fg',
      ]"
      @click="!disabled && emit('click')"
      @keydown="pressOnKey"
      @keyup="pressOnKey"
    >
      <component
        :is="icon"
        v-if="icon"
        :size="phone ? ICON_PX.phoneRow : ICON_PX.in28"
        class="shrink-0"
      />
      <span class="min-w-0 flex-1 truncate">{{ label }}</span>
      <span
        v-if="shortcut"
        class="text-[11px] text-fg-faint phone:hidden"
      >{{ shortcut }}</span>
    </div>
  </Tooltip>
</template>
