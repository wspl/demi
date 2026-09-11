<script setup lang="ts">
import { computed, ref, useAttrs } from 'vue'
import { blockUnavailableButtonEvent } from './button-events'
import { useButtonIconSpin } from './button-icon-spin'
import type { Component } from 'vue'
import { disabledTooltip } from './disabled'
import { ICON_PX } from './icon-metrics'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import Tooltip from './Tooltip.vue'

defineOptions({ inheritAttrs: false })
const attrs = useAttrs()
const restAttrs = computed(() => {
  const next: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(attrs)) {
    if (key !== 'class') {
      next[key] = value
    }
  }
  return next
})

const props = withDefaults(
  defineProps<{
    icon: Component
    iconSize?: number
    size?: 'xs' | 'sm' | 'md' | 'lg'
    variant?: 'default' | 'solid' | 'ghost' | 'danger' | 'accent'
    circle?: boolean
    disabled?: boolean
    loading?: boolean
    /** Why it is disabled, as a tooltip; only read while `disabled`. */
    disabledReason?: string
    pressed?: boolean
    spinning?: boolean
    spinOnClick?: boolean
  }>(),
  {
    size: 'md',
    variant: 'default',
  },
)

const pressed = computed(() => props.pressed === true)
const tooltipContent = computed(() =>
  disabledTooltip(props.disabled, props.disabledReason),
)
const emit = defineEmits<{ spinEnd: [] }>()
const root = ref<HTMLElement | null>(null)
const { rotating, onClick } = useButtonIconSpin(root, props, () =>
  emit('spinEnd'),
)

const glyphPx = computed(() => {
  if (props.iconSize != null) {
    return props.iconSize
  }
  if (props.size === 'xs') {
    return ICON_PX.in20
  }
  if (props.size === 'lg') {
    return ICON_PX.in32
  }
  return ICON_PX.in28
})
</script>

<template>
  <Tooltip
    :content="tooltipContent"
    :disabled="!tooltipContent"
    tag="span"
    class="inline-flex"
    :class="attrs.class"
    :open-delay-ms="80"
  >
    <span
      ref="root"
      v-bind="restAttrs"
      :data-spinning="rotating || undefined"
      @click="onClick"
      @click.capture="blockUnavailableButtonEvent($event, disabled || loading)"
      @keydown.capture="
        blockUnavailableButtonEvent($event, disabled || loading)
      "
      role="button"
      class="inline-flex shrink-0 cursor-default items-center justify-center transition-[color,background-color,box-shadow,filter] duration-200 ease-out"
      :aria-disabled="disabled || loading || undefined"
      :aria-busy="loading || undefined"
      :data-pressed="!disabled && pressed ? true : undefined"
      :class="[
        circle ? 'rounded-full' : 'rounded-md',
        size === 'xs'
          ? 'size-hit-xs'
          : size === 'sm'
            ? 'size-hit-sm'
            : size === 'lg'
              ? 'size-hit-lg'
              : 'size-hit',
        disabled
          ? 'pointer-events-none cursor-not-allowed text-fg-ghost'
          : variant === 'accent'
            ? [
                'btn-primary text-white',
                pressed ? 'brightness-110' : 'hover:brightness-110',
              ]
            : variant === 'ghost'
              ? circle
                ? pressed
                  ? 'bg-active text-fg-body'
                  : 'bg-hover text-fg-muted hover:bg-active hover:text-fg-body'
                : pressed
                  ? 'bg-hover text-fg-body'
                  : 'text-fg-muted hover:bg-hover hover:text-fg-body'
              : variant === 'danger'
                ? 'btn text-on-danger'
                : variant === 'solid'
                  ? 'btn-solid text-fg-body'
                  : 'btn text-fg-body',
      ]"
    >
      <IndeterminateSpinner v-if="loading" :size="glyphPx" />
      <component
        v-else
        :is="icon"
        :size="glyphPx"
        :width="glyphPx"
        :height="glyphPx"
      />
    </span>
  </Tooltip>
</template>
