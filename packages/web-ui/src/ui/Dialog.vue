<script setup lang="ts">
import { computed, inject, provide } from 'vue'
import { X } from '@lucide/vue'
import IconButton from './IconButton.vue'
import ScrollArea from './ScrollArea.vue'
import { onKeyStroke } from '@vueuse/core'
import { createOverlayFamily, overlayFamilyKey } from '../overlay/overlayFamily'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayContainerKey } from '../overlay/overlayContainer'
import { useOverlay } from '../composables/useOverlay'

/**
 * Scrolling: a dialog's header, search and footer stay put; only its body scrolls.
 * The panel is a flex column capped at the host's height, so content declares one
 * root with `flex min-h-0 flex-col` and puts the body in a `ScrollArea`. Content
 * without that root simply scrolls as a whole.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  size?: 'md' | 'lg' | 'xl'
  label?: string
  /** Every dialog closes from its top-right corner; a flow that must finish can hide it. */
  hideClose?: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

// A dialog confined to a host container never blocks the page, so it is not exclusive.
const container = inject(overlayContainerKey, null)
const teleportTarget = computed(() => container?.value ?? 'body')
const family = createOverlayFamily()
// Child menus share ownership, but the dialog surface is outside their click boundary.
provide(overlayFamilyKey, family)
onKeyStroke('Escape', (event) => {
  if (!props.isOpen || container) return
  event.preventDefault()
  emit('close')
})

useOverlay(props.overlayStore, () => (container ? false : props.isOpen), () => {
  if (props.isOpen) emit('close')
})
</script>

<template>
  <Teleport :to="teleportTarget">
    <!-- One transition for scrim and panel: a nested one never gets to leave, since the
         outer v-if unmounts the subtree. The panel's scale rides on the same stage classes. -->
    <Transition name="dialog" appear>
      <div
        v-if="isOpen"
        class="dialog-scrim fixed inset-0 z-50 grid place-items-center bg-black/40"
        @click.self="emit('close')"
      >
        <div
          class="dialog-panel relative flex max-h-[calc(100%-2rem)] w-[calc(100%-2rem)] flex-col overflow-hidden rounded-xl bg-surface-dialog shadow-2xl"
          :class="size === 'xl' ? 'max-w-5xl' : size === 'lg' ? 'max-w-3xl' : 'max-w-md'"
          role="dialog"
          aria-modal="true"
          :aria-label="label"
        >
          <div v-if="!hideClose" class="absolute right-3 top-3 z-10">
            <IconButton :icon="X" variant="ghost" aria-label="Close" @click="emit('close')" />
          </div>
          <!-- Content with its own scrolling body shrinks inside; anything else scrolls as a whole. -->
          <ScrollArea class="min-h-0" viewport-class="flex flex-col">
            <slot />
          </ScrollArea>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.dialog-enter-active,
.dialog-leave-active {
  transition: opacity 150ms ease-out;
}

.dialog-enter-active .dialog-panel,
.dialog-leave-active .dialog-panel {
  transition: opacity 150ms ease-out, transform 150ms ease-out;
}

.dialog-enter-from,
.dialog-leave-to {
  opacity: 0;
}

.dialog-enter-from .dialog-panel,
.dialog-leave-to .dialog-panel {
  opacity: 0;
  transform: scale(0.95);
}

@media (prefers-reduced-motion: reduce) {
  .dialog-enter-active,
  .dialog-leave-active,
  .dialog-enter-active .dialog-panel,
  .dialog-leave-active .dialog-panel {
    transition: none;
  }
}
</style>
