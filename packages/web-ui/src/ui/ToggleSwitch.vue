<script setup lang="ts">
import Tooltip from './Tooltip.vue'

defineProps<{
  modelValue: boolean
  tooltip?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
}>()
</script>

<template>
  <Tooltip :content="tooltip" class="inline-flex w-fit max-w-max">
    <div
      class="inline-flex w-fit shrink-0 items-stretch rounded-md bg-overlay/6 p-[2px]"
      role="radiogroup"
    >
      <span
        role="radio"
        :aria-checked="!modelValue"
        class="inline-flex cursor-default items-center justify-center rounded-[5px] px-1.5 py-0.5 text-[11px] leading-4 whitespace-nowrap transition-[color,background-color,box-shadow] duration-200 ease-out select-none"
        :class="!modelValue
          ? 'text-fg-emphasis toggle-switch-thumb'
          : 'text-fg-subtle hover:text-fg'"
        @click="emit('update:modelValue', false)"
      >
        <slot name="left" />
      </span>
      <span
        role="radio"
        :aria-checked="modelValue"
        class="inline-flex cursor-default items-center justify-center rounded-[5px] px-1.5 py-0.5 text-[11px] leading-4 whitespace-nowrap transition-[color,background-color,box-shadow] duration-200 ease-out select-none"
        :class="modelValue
          ? 'text-fg-emphasis toggle-switch-thumb'
          : 'text-fg-subtle hover:text-fg'"
        @click="emit('update:modelValue', true)"
      >
        <slot name="right" />
      </span>
    </div>
  </Tooltip>
</template>

<style scoped>
.toggle-switch-thumb {
  background: var(--btn-bg);
  box-shadow: var(--shadow-btn);
}

html[data-theme="dark"] .toggle-switch-thumb {
  background: color-mix(in srgb, var(--color-overlay) 12%, var(--color-surface));
}
</style>
