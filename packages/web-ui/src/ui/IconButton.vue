<script setup lang="ts">
import type { Component } from 'vue'

withDefaults(defineProps<{
  icon: Component
  iconSize?: number
  size?: 'xs' | 'sm' | 'md' | 'lg'
  variant?: 'default' | 'danger'
  disabled?: boolean
}>(), {
  size: 'md',
  variant: 'default',
})
</script>

<template>
  <span
    class="flex shrink-0 cursor-pointer items-center justify-center transition-colors"
    :class="[
      size === 'xs' ? 'size-4 rounded-sm' : size === 'sm' ? 'size-5 rounded' : size === 'lg' ? 'size-8 rounded-lg' : 'size-6 rounded-md',
      disabled
        ? 'pointer-events-none text-fg-ghost'
        : variant === 'danger'
          ? 'text-fg-subtle hover:bg-hover hover:text-on-danger'
          : 'text-fg-subtle hover:bg-hover hover:text-fg-body',
    ]"
  >
    <component :is="icon" :size="iconSize ?? (size === 'xs' ? 10 : size === 'sm' ? 14 : size === 'lg' ? 18 : 16)" />
  </span>
</template>
