<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(defineProps<{
  size?: 'xs' | 'sm' | 'md'
  variant?: 'default' | 'primary' | 'ghost'
  disabled?: boolean
  pressed?: boolean
}>(), {
  size: 'md',
  variant: 'default',
})

const pressed = computed(() => props.pressed === true)
</script>

<template>
  <span
    role="button"
    class="inline-flex cursor-default items-center justify-center gap-1 rounded-md transition-[color,background-color,box-shadow,filter] duration-200 ease-out select-none"
    :data-pressed="!disabled && pressed ? true : undefined"
    :class="[
      size === 'md' ? 'h-7 px-2.5 text-chrome' : size === 'xs' ? 'h-5 px-1.5 text-[11px]' : 'h-6 px-2 text-[12px]',
      variant === 'primary'
        ? ['btn-primary font-medium text-white', pressed ? 'brightness-110' : 'hover:brightness-110']
        : variant === 'ghost'
          ? pressed
            ? 'font-normal bg-hover text-fg-body'
            : 'font-normal text-fg-muted hover:bg-hover hover:text-fg-body'
          : ['btn font-medium text-fg-body', pressed ? 'text-fg-emphasis' : 'hover:text-fg-emphasis'],
      disabled ? 'pointer-events-none opacity-40' : '',
    ]"
  >
    <slot />
  </span>
</template>
