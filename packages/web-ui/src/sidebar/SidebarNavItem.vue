<script setup lang="ts">
import { computed } from 'vue'
import type { Component } from 'vue'
import { disabledTooltip } from '@demicodes/web-ui/ui/disabled'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'

/**
 * One entry outside the list: a primary action or a management surface (plugins, skills,
 * settings), or a place in a rail.
 */
const props = defineProps<{
  icon?: Component
  label: string
  shortcut?: string
  count?: number
  pressed?: boolean
  emphasis?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: string
}>()

const tooltipContent = computed(() => disabledTooltip(props.disabled, props.disabledReason))

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
      class="flex h-7 cursor-default select-none items-center gap-2 rounded-md text-chrome transition-colors duration-200 ease-out"
      :class="[
        'px-2',
        disabled
          ? 'cursor-not-allowed text-fg-faint'
          : pressed
            ? 'bg-active text-fg-emphasis'
            : emphasis
              ? 'text-fg hover:bg-hover hover:text-fg-emphasis'
              : 'text-fg-muted hover:bg-hover hover:text-fg',
      ]"
      @click="!disabled && emit('click')"
    >
      <component
        :is="icon"
        v-if="icon"
        :size="ICON_PX.in28"
        class="shrink-0"
      />
      <span class="min-w-0 flex-1 truncate">{{ label }}</span>
      <span
        v-if="count !== undefined"
        class="rounded-full bg-hover px-1.5 text-[11px] leading-4 text-fg-subtle"
      >
        {{ count }}
      </span>
      <span
        v-else-if="shortcut"
        class="text-[11px] text-fg-faint [@media(hover:none)]:hidden"
      >{{ shortcut }}</span>
    </div>
  </Tooltip>
</template>
