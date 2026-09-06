<script setup lang="ts">
import type { Component } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/** One entry in a SettingsSplit list: a mark or icon, the name, and a badge when something is wrong. */
defineProps<{
  label: string
  detail?: string
  icon?: Component
  selected?: boolean
  /** A trouble badge on the mark's corner. Healthy entries show nothing. */
  badge?: 'warning' | 'danger'
  muted?: boolean
}>()

const emit = defineEmits<{
  select: []
}>()
</script>

<template>
  <div
    role="button"
    :aria-pressed="selected"
    class="flex h-8 cursor-default select-none items-center gap-2 rounded-md px-1 transition-colors duration-200 ease-out"
    :class="[selected ? 'bg-active' : 'hover:bg-hover', muted ? 'opacity-60' : '']"
    @click="emit('select')"
  >
    <span class="relative flex shrink-0 items-center">
      <slot name="leading">
        <component :is="icon" v-if="icon" :size="ICON_PX.in28" class="text-fg-muted" />
      </slot>
      <span
        v-if="badge"
        class="absolute -bottom-0.5 -right-0.5 size-2 rounded-full ring-2 ring-surface"
        :class="badge === 'danger' ? 'bg-on-danger' : 'bg-on-warning'"
      />
    </span>
    <span class="flex min-w-0 flex-1 flex-col leading-4">
      <span class="truncate text-chrome" :class="selected ? 'text-fg-emphasis' : 'text-fg'">{{ label }}</span>
      <span v-if="detail" class="truncate text-[11px] text-fg-subtle">{{ detail }}</span>
    </span>
  </div>
</template>
