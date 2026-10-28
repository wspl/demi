<script setup lang="ts">
/**
 * A workspace's icon and color: the colors in a row over the icons, the
 * chosen ones marked. A choice applies at once, and the picker stays open so
 * the other can be chosen too, as a color well's panel does.
 */
import { pressOnKey } from '../ui/button-events'
import { WORKSPACE_COLORS, WORKSPACE_ICONS, type WorkspaceColor, type WorkspaceIcon } from './workspaces'

defineProps<{
  icon: WorkspaceIcon
  color: WorkspaceColor
}>()

const emit = defineEmits<{
  change: [change: { icon?: WorkspaceIcon; color?: WorkspaceColor }]
}>()
</script>

<template>
  <div class="overlay-shell w-64 rounded-lg p-3" style="--elevation: 1" role="group">
    <div class="flex justify-between px-1 pb-3" role="radiogroup">
      <span
        v-for="entry in WORKSPACE_COLORS"
        :key="entry.id"
        role="radio"
        tabindex="0"
        :aria-checked="entry.id === color"
        :aria-label="entry.id"
        class="size-5 cursor-default rounded-full outline-offset-2"
        :style="{
          background: entry.fill,
          boxShadow: entry.id === color ? `0 0 0 2px var(--color-surface-float), 0 0 0 4px ${entry.fill}` : undefined,
        }"
        @click="emit('change', { color: entry.id })"
        @keydown="pressOnKey"
        @keyup="pressOnKey"
      />
    </div>
    <div class="-mx-3 mb-2 h-px bg-line-subtle" />
    <div class="grid grid-cols-6 justify-between gap-y-1" role="radiogroup">
      <span
        v-for="entry in WORKSPACE_ICONS"
        :key="entry.id"
        role="radio"
        tabindex="0"
        :aria-checked="entry.id === icon"
        :aria-label="entry.id"
        class="flex size-8 cursor-default items-center justify-center rounded-md transition-colors duration-200 ease-out"
        :class="entry.id === icon ? 'bg-active text-fg-emphasis' : 'text-fg-muted hover:bg-hover hover:text-fg-body'"
        @click="emit('change', { icon: entry.id })"
        @keydown="pressOnKey"
        @keyup="pressOnKey"
      >
        <component :is="entry.component" :size="18" />
      </span>
    </div>
  </div>
</template>
