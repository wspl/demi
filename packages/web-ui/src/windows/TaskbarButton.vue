<script setup lang="ts">
import { onBeforeUpdate, onUpdated, useTemplateRef, type Component } from 'vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { TaskbarItem } from './Taskbar.vue'
import { resized } from './taskbar-motion'

/**
 * One button of the taskbar: its icon, then its name, then a count. A
 * window's button shows its tab's name and, holding more than one tab,
 * their count; a system button, its count alone; More, its name.
 *
 * The state says how it draws: closed and minimized windows are fainter; a
 * shown window wears a mark under it, the one in front the accent's, as a menu
 * the button opens does while it is open.
 */
defineProps<{
  icon: Component
  label?: string
  count?: number
  state?: TaskbarItem['state']
  /** Keeps its width while the taskbar narrows its windows' buttons. */
  fixed?: boolean
}>()

// A new name or count takes the button to its new width with motion.
const button = useTemplateRef<HTMLButtonElement>('button')
let before = 0
onBeforeUpdate(() => {
  before = button.value?.getBoundingClientRect().width ?? 0
})
onUpdated(() => {
  if (button.value && before > 0) {
    resized(button.value, before)
  }
})
</script>

<template>
  <button
    ref="button"
    type="button"
    class="taskbar-button"
    :class="[state ? `taskbar-${state}` : '', { 'taskbar-fixed': fixed }]"
  >
    <component :is="icon" :size="ICON_PX.chip" class="shrink-0" />
    <span v-if="label" class="min-w-0 truncate">{{ label }}</span>
    <span v-if="count !== undefined" class="taskbar-count">{{ count }}</span>
  </button>
</template>

<style scoped>
.taskbar-button {
  position: relative;
  display: inline-flex;
  height: 32px;
  flex: 0 1 auto;
  align-items: center;
  gap: 6px;
  overflow: hidden;
  border-radius: 8px;
  padding-inline: 9px;
  white-space: nowrap;
  font-size: var(--text-chrome);
  color: var(--color-fg);
  cursor: default;
}

.taskbar-button:hover {
  background: var(--color-surface-base);
}

.taskbar-fixed {
  flex: none;
}

.taskbar-closed {
  color: var(--color-fg-subtle);
}

.taskbar-minimized {
  color: var(--color-fg-muted);
}

.taskbar-front {
  background: var(--color-surface-base);
  color: var(--color-fg-emphasis);
}

/* Shown windows wear a mark under their button, the one in front the accent's. */
.taskbar-shown::after,
.taskbar-front::after {
  content: "";
  position: absolute;
  left: 50%;
  bottom: 2px;
  height: 3px;
  width: 10px;
  margin-left: -5px;
  border-radius: 2px;
  background: var(--color-fg-faint);
}

.taskbar-front::after {
  width: 16px;
  margin-left: -8px;
  background: var(--color-accent-fill);
}

.taskbar-count {
  display: inline-grid;
  min-width: 16px;
  height: 16px;
  flex: none;
  place-items: center;
  border-radius: 8px;
  padding-inline: 4px;
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  background: var(--color-surface-raised);
  color: var(--color-fg-muted);
}
</style>
