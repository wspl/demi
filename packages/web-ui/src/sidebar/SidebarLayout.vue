<script setup lang="ts">
import { computed } from 'vue'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { clampSize } from '../ui/resize-handle'
import { SIDEBAR_WIDTH } from './sidebar-width'

/**
 * The app frame: the sidebar, the divider that sizes it, and the main pane.
 * The host keeps the width (v-model) and hears `resizeEnd` when a size has
 * settled, which is the moment to persist it. Below the medium breakpoint the
 * sidebar is an overlay at the same width and the divider is gone.
 */
withDefaults(defineProps<{ label?: string }>(), { label: 'Demi' })
const emit = defineEmits<{ resizeEnd: [width: number] }>()
const open = defineModel<boolean>('open', { default: false })
const width = defineModel<number>('width', { default: SIDEBAR_WIDTH.default })
// A stored width outside today's bounds is shown at the bound, not rewritten.
const shownWidth = computed(() => clampSize(width.value, SIDEBAR_WIDTH))
</script>

<template>
  <div class="flex h-full bg-surface-base text-fg">
    <div
      v-if="open"
      class="fixed inset-0 z-30 bg-black/50 md:hidden"
      @click="open = false"
    />
    <div
      class="h-full shrink-0"
      :class="open ? 'fixed inset-y-0 left-0 z-40 md:static' : 'hidden md:block'"
      :style="{ '--sidebar-width': `${shownWidth}px` }"
    >
      <slot name="sidebar" />
    </div>
    <ResizeHandle
      v-model="width"
      class="hidden md:block"
      :min="SIDEBAR_WIDTH.min"
      :max="SIDEBAR_WIDTH.max"
      :default-value="SIDEBAR_WIDTH.default"
      label="Sidebar width"
      @commit="emit('resizeEnd', $event)"
    />
    <main class="flex min-w-0 flex-1 flex-col overflow-hidden">
      <div class="flex select-none items-center px-2 md:hidden">
        <IconButton
          :icon="PanelLeft"
          variant="ghost"
          aria-label="Open sidebar"
          @click="open = true"
        />
        <span class="px-2 text-chrome text-fg-muted">{{ label }}</span>
      </div>
      <slot />
    </main>
    <slot name="dialogs" />
  </div>
</template>
