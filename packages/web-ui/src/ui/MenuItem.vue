<script setup lang="ts">
import { computed } from 'vue'
import type { Component } from 'vue'
import Tooltip from './Tooltip.vue'

const props = defineProps<{
  icon?: Component
  label: string
  isDanger?: boolean
  disabled?: boolean
  disabledReason?: string
  shortcut?: string
}>()

const emit = defineEmits<{
  select: []
}>()

const displayShortcut = computed(() => props.shortcut ?? '')
const isDisabled = computed(() => props.disabled || !!props.disabledReason)
const tooltipContent = computed(() => props.disabledReason?.trim() || undefined)

function handleClick(event: MouseEvent) {
  if (isDisabled.value) {
    event.stopPropagation()
    return
  }
  emit('select')
}
</script>

<template>
  <Tooltip :content="tooltipContent" :disabled="!tooltipContent" placement="bottom" :open-delay-ms="80" tag="div">
    <div
      class="flex select-none items-center gap-2 rounded-md px-2.5 py-1.5 text-[13px] transition-colors"
      :class="isDisabled
        ? 'cursor-not-allowed text-fg-faint'
        : isDanger
          ? 'cursor-pointer text-on-danger hover:bg-tint-danger-strong hover:text-on-danger'
          : 'cursor-pointer text-fg-body hover:bg-active hover:text-fg-emphasis'"
      @click="handleClick"
    >
      <component :is="icon" v-if="icon" :size="15" class="shrink-0" />
      <span class="flex-1">{{ label }}</span>
      <span class="w-8 shrink-0 text-right text-[11px] text-fg-faint">{{ displayShortcut }}</span>
    </div>
  </Tooltip>
</template>
