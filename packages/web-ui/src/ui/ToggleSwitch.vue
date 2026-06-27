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
  <Tooltip :content="tooltip">
    <div
      class="flex cursor-pointer items-center gap-0.5 rounded bg-surface-raised p-0.5 ring-1 ring-line-subtle hover:ring-line-strong"
      @click="emit('update:modelValue', !modelValue)"
    >
      <span
        class="flex items-center rounded-sm p-1 transition-colors"
        :class="!modelValue ? 'bg-fg-ghost text-fg' : 'text-fg-subtle'"
      >
        <slot name="left" />
      </span>
      <span
        class="flex items-center rounded-sm p-1 transition-colors"
        :class="modelValue ? 'bg-fg-ghost text-fg' : 'text-fg-subtle'"
      >
        <slot name="right" />
      </span>
    </div>
  </Tooltip>
</template>
