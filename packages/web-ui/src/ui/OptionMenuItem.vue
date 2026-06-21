<script setup lang="ts">
import type { Component } from 'vue'
import { CheckLine } from '@mingcute/vue/check'

defineProps<{
  icon?: Component
  label: string
  isSelected?: boolean
}>()

const emit = defineEmits<{
  select: []
}>()
</script>

<template>
  <div
    class="flex cursor-pointer items-center gap-2 rounded-md px-2.5 py-1.5 text-[13px] transition-colors"
    :class="isSelected
      ? 'bg-active text-fg-emphasis'
      : 'text-fg-muted hover:bg-hover hover:text-fg'"
    @click="emit('select')"
  >
    <component :is="icon" v-if="icon" :size="15" class="shrink-0" />
    <span class="flex-1">{{ label }}</span>
    <CheckLine v-if="isSelected" :size="14" class="shrink-0 text-fg-body" />
  </div>
</template>
