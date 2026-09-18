<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { clampSize } from '../ui/resize-handle'
import { ASIDE_SHARE, SIDEBAR_WIDTH, asideBounds, asideShareFor, asideWidthFor } from './sidebar-width'

/**
 * The app frame: the sidebar, the main pane, and the work panel (aside) on
 * the right, with a divider sizing each side pane. The sidebar keeps a px
 * width; the aside keeps a share of the width it splits with the main pane,
 * so a window resize scales the two together and leaves the sidebar alone.
 * The host keeps both sizes (v-model) and hears `resizeEnd` /
 * `asideResizeEnd` when one has settled, which is the moment to persist it.
 * The aside is only there while `asideOpen` is true and the host gave the slot.
 *
 * Below the medium breakpoint each side pane is an overlay at the same width
 * with no divider, and only one of them is open at a time: opening one closes
 * the other, and the scrim closes whichever is open.
 */
withDefaults(defineProps<{ label?: string }>(), { label: 'Demi' })
const emit = defineEmits<{
  resizeEnd: [width: number]
  asideResizeEnd: [share: number]
}>()
const open = defineModel<boolean>('open', { default: false })
const width = defineModel<number>('width', { default: SIDEBAR_WIDTH.default })
const asideOpen = defineModel<boolean>('asideOpen', { default: false })
const asideShare = defineModel<number>('asideShare', { default: ASIDE_SHARE.default })
// A stored width outside today's bounds is shown at the bound, not rewritten.
const shownWidth = computed(() => clampSize(width.value, SIDEBAR_WIDTH))

// The main pane and the aside split what the sidebar leaves; a hidden sidebar measures 0.
const frame = ref<HTMLElement | null>(null)
const sidebar = ref<HTMLElement | null>(null)
const { width: frameWidth } = useElementSize(frame, { width: window.innerWidth, height: 0 })
const { width: sidebarWidth } = useElementSize(sidebar)
const sharedWidth = computed(() => frameWidth.value - sidebarWidth.value)
const shownAsideWidth = computed(() => asideWidthFor(asideShare.value, sharedWidth.value))
const asideDragBounds = computed(() => asideBounds(sharedWidth.value))

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
  <div ref="frame" class="flex h-full bg-surface-base text-fg">
    <div
      v-if="open || asideOpen"
      class="fixed inset-0 z-30 bg-black/50 md:hidden"
      @click="closeOverlays"
    />
    <div
      ref="sidebar"
      class="h-full shrink-0"
      :class="open ? 'fixed inset-y-0 left-0 z-40 md:static md:z-auto' : 'hidden md:block'"
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
      <!-- The handle works in px; the model keeps the share those px are of the shared width. -->
      <ResizeHandle
        :model-value="shownAsideWidth"
        class="hidden md:block"
        side="end"
        :min="asideDragBounds.min"
        :max="asideDragBounds.max"
        :default-value="asideWidthFor(ASIDE_SHARE.default, sharedWidth)"
        label="Work panel width"
        @update:model-value="asideShare = asideShareFor($event, sharedWidth)"
        @commit="emit('asideResizeEnd', asideShareFor($event, sharedWidth))"
      />
      <!-- A flex item honors z-index even when static, so the overlay's z-40 must end at md:
           otherwise the panel covers the right half of the resize handle beside it. -->
      <div
        class="fixed inset-y-0 right-0 z-40 h-full max-w-full shrink-0 md:static md:z-auto"
        :style="{ width: `${shownAsideWidth}px` }"
      >
        <slot name="aside" />
      </div>
    </template>
    <slot name="dialogs" />
  </div>
</template>
