<script setup lang="ts">
import { computed, watch } from 'vue'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { clampSize } from '../ui/resize-handle'
import { ASIDE_WIDTH, SIDEBAR_WIDTH } from './sidebar-width'

/**
 * The app frame: the sidebar, the main pane, and the work panel (aside) on
 * the right, with a divider sizing each side pane. The host keeps both widths
 * (v-model) and hears `resizeEnd` / `asideResizeEnd` when a size has settled,
 * which is the moment to persist it. The aside is only there while
 * `asideOpen` is true and the host gave the slot.
 *
 * Below the medium breakpoint each side pane is an overlay at the same width
 * with no divider, and only one of them is open at a time: opening one closes
 * the other, and the scrim closes whichever is open.
 */
withDefaults(defineProps<{ label?: string }>(), { label: 'Demi' })
const emit = defineEmits<{
  resizeEnd: [width: number]
  asideResizeEnd: [width: number]
}>()
const open = defineModel<boolean>('open', { default: false })
const width = defineModel<number>('width', { default: SIDEBAR_WIDTH.default })
const asideOpen = defineModel<boolean>('asideOpen', { default: false })
const asideWidth = defineModel<number>('asideWidth', { default: ASIDE_WIDTH.default })
// A stored width outside today's bounds is shown at the bound, not rewritten.
const shownWidth = computed(() => clampSize(width.value, SIDEBAR_WIDTH))
const shownAsideWidth = computed(() => clampSize(asideWidth.value, ASIDE_WIDTH))

function isNarrow(): boolean {
  return window.matchMedia('(max-width: 767px)').matches
}

// On a narrow screen the two overlays never stack: the newly opened one wins.
watch(open, (value) => {
  if (value && isNarrow()) {
    asideOpen.value = false
  }
})
watch(asideOpen, (value) => {
  if (value && isNarrow()) {
    open.value = false
  }
})

function closeOverlays(): void {
  open.value = false
  asideOpen.value = false
}
</script>

<template>
  <div class="flex h-full bg-surface-base text-fg">
    <div
      v-if="open || asideOpen"
      class="fixed inset-0 z-30 bg-black/50 md:hidden"
      @click="closeOverlays"
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
    <template v-if="asideOpen && $slots.aside">
      <ResizeHandle
        v-model="asideWidth"
        class="hidden md:block"
        side="end"
        :min="ASIDE_WIDTH.min"
        :max="ASIDE_WIDTH.max"
        :default-value="ASIDE_WIDTH.default"
        label="Work panel width"
        @commit="emit('asideResizeEnd', $event)"
      />
      <div
        class="fixed inset-y-0 right-0 z-40 h-full max-w-full shrink-0 md:static"
        :style="{ width: `${shownAsideWidth}px` }"
      >
        <slot name="aside" />
      </div>
    </template>
    <slot name="dialogs" />
  </div>
</template>
