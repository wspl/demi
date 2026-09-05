<script setup lang="ts">
import type { Component } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/** One entry outside the list: a primary action or a management surface (plugins, skills, settings). */
defineProps<{
  icon: Component
  label: string
  shortcut?: string
  count?: number
  pressed?: boolean
  emphasis?: boolean
}>()

const emit = defineEmits<{
  click: []
}>()
</script>

<template>
  <div
    role="button"
    class="flex h-7 cursor-default select-none items-center gap-2 rounded-md text-chrome transition-colors duration-200 ease-out"
    :class="[
      'px-2',
      pressed
        ? 'bg-active text-fg-emphasis'
        : emphasis
          ? 'text-fg hover:bg-hover hover:text-fg-emphasis'
          : 'text-fg-muted hover:bg-hover hover:text-fg',
    ]"
    @click="emit('click')"
  >
    <component :is="icon" :size="ICON_PX.in28" class="shrink-0" />
    <span class="min-w-0 flex-1 truncate">{{ label }}</span>
    <span
      v-if="count !== undefined"
      class="rounded-full bg-hover px-1.5 text-[11px] leading-4 text-fg-subtle"
    >
      {{ count }}
    </span>
    <span v-else-if="shortcut" class="text-[11px] text-fg-faint">{{ shortcut }}</span>
  </div>
</template>
