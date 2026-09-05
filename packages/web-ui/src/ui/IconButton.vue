<script setup lang="ts">
import { computed } from 'vue'
import type { Component } from 'vue'
import { ICON_PX } from './icon-metrics'

const props = withDefaults(defineProps<{
  icon: Component
  iconSize?: number
  size?: 'xs' | 'sm' | 'md' | 'lg'
  variant?: 'default' | 'ghost' | 'danger' | 'accent'
  circle?: boolean
  disabled?: boolean
  pressed?: boolean
}>(), {
  size: 'md',
  variant: 'default',
})

const pressed = computed(() => props.pressed === true)

const glyphPx = computed(() => {
  if (props.iconSize != null) return props.iconSize
  if (props.size === 'xs') return ICON_PX.in20
  if (props.size === 'lg') return ICON_PX.in32
  return ICON_PX.in28
})
</script>

<template>
  <span
    role="button"
    class="inline-flex shrink-0 cursor-default items-center justify-center transition-[color,background-color,box-shadow,filter] duration-200 ease-out"
    :data-pressed="!disabled && pressed ? true : undefined"
    :class="[
      circle ? 'rounded-full' : 'rounded-md',
      size === 'xs' ? 'size-hit-xs' : size === 'sm' ? 'size-hit-sm' : size === 'lg' ? 'size-hit-lg' : 'size-hit',
      disabled
        ? 'pointer-events-none text-fg-ghost'
        : variant === 'accent'
          ? ['btn-primary text-white', pressed ? 'brightness-110' : 'hover:brightness-110']
          : variant === 'ghost'
            ? circle
              ? pressed
                ? 'bg-active text-fg-body'
                : 'bg-hover text-fg-muted hover:bg-active hover:text-fg-body'
              : pressed
                ? 'bg-hover text-fg-body'
                : 'text-fg-muted hover:bg-hover hover:text-fg-body'
            : variant === 'danger'
              ? ['btn text-fg-body', pressed ? 'text-on-danger' : 'hover:text-on-danger']
              : 'btn text-fg-body',
    ]"
  >
    <component :is="icon" :size="glyphPx" :width="glyphPx" :height="glyphPx" />
  </span>
</template>
