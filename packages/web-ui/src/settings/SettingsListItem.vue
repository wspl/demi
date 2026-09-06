<script setup lang="ts">
import type { Component } from 'vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'

/** One entry in a SettingsSplit list: icon, name, a word under it, and a presence dot. */
defineProps<{
  label: string
  detail?: string
  icon?: Component
  selected?: boolean
  /** Presence dot tone; omit for none. */
  dot?: 'success' | 'warning' | 'danger' | 'neutral'
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
    class="flex h-11 cursor-default select-none items-center gap-2.5 rounded-md px-2 transition-colors duration-200 ease-out"
    :class="[selected ? 'bg-active' : 'hover:bg-hover', muted ? 'opacity-60' : '']"
    @click="emit('select')"
  >
    <component :is="icon" v-if="icon" :size="ICON_PX.in28" class="shrink-0 text-fg-muted" />
    <span class="flex min-w-0 flex-1 flex-col leading-4">
      <span class="truncate text-chrome" :class="selected ? 'text-fg-emphasis' : 'text-fg'">{{ label }}</span>
      <span v-if="detail" class="truncate text-[11px] text-fg-subtle">{{ detail }}</span>
    </span>
    <span
      v-if="dot"
      class="size-1.5 shrink-0 rounded-full"
      :class="{
        'bg-on-success': dot === 'success',
        'bg-on-warning': dot === 'warning',
        'bg-on-danger': dot === 'danger',
        'bg-fg-ghost': dot === 'neutral',
      }"
    />
  </div>
</template>
