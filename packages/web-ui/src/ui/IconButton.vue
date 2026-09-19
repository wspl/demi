<script setup lang="ts">
import { computed, ref, useAttrs } from 'vue'
import { blockUnavailableButtonEvent } from './button-events'
import { useButtonIconSpin } from './button-icon-spin'
import type { Component } from 'vue'
import { disabledTooltip } from './disabled'
import { ICON_PX } from './icon-metrics'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import CornerDot, { type CornerDotTone } from './CornerDot.vue'
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
    /** Turns the icon while true, a whole revolution at a time; the turn it is in always finishes. */
    spinning?: boolean
    /** Turns the icon one whole revolution per click. Every button that refreshes, renews or restarts does. */
    spinOnClick?: boolean
    /** A status on the icon's top-right corner: a small dot in this tone, none when null. */
    indicator?: CornerDotTone | null
    /** What the dot means, for assistive technology; decorative without it. */
    indicatorLabel?: string
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
// The icon's own box turns, not the dot on its corner.
const glyph = ref<HTMLElement | null>(null)
const { rotating, onClick } = useButtonIconSpin(
  () => glyph.value ? [glyph.value] : [],
  props,
  () => emit('spinEnd'),
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
      <span v-else class="relative flex">
        <span ref="glyph" class="flex">
          <component
            :is="icon"
            :size="glyphPx"
            :width="glyphPx"
            :height="glyphPx"
          />
        </span>
        <CornerDot
          v-if="indicator !== undefined"
          :tone="indicator"
          size="xs"
          :ring="variant === 'ghost' ? 'surface' : 'button'"
          :label="indicatorLabel"
        />
      </span>
    </span>
  </Tooltip>
</template>
