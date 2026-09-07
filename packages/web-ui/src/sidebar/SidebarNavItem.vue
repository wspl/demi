<script setup lang="ts">
import type { Component } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/**
 * One entry outside the list: a primary action or a management surface (plugins, skills,
 * settings), or a place in a rail. A status dot may stand in for the icon.
 */
defineProps<{
  icon?: Component
  label: string
  indicator?: 'success' | 'muted'
  indicatorLabel?: string
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
    :aria-label="label"
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
    <span v-if="indicator" class="flex size-3.5 shrink-0 items-center justify-center">
      <span
        class="size-1.5 rounded-full"
        :class="indicator === 'success' ? 'bg-on-success' : 'bg-fg-faint'"
        role="img"
        :aria-label="indicatorLabel"
      />
    </span>
    <component :is="icon" v-else-if="icon" :size="ICON_PX.in28" class="shrink-0" />
    <span class="min-w-0 flex-1 truncate">{{ label }}</span>
    <span
      v-if="count !== undefined"
      class="rounded-full bg-hover px-1.5 text-[11px] leading-4 text-fg-subtle"
    >
      {{ count }}
    </span>
    <span v-else-if="shortcut" class="text-[11px] text-fg-faint [@media(hover:none)]:hidden">{{ shortcut }}</span>
  </div>
</template>
