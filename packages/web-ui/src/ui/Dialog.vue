<script setup lang="ts">
import { computed, inject, provide } from 'vue'
import { X } from '@lucide/vue'
import IconButton from './IconButton.vue'
import ScrollArea from './ScrollArea.vue'
import { onKeyStroke } from '@vueuse/core'
import { createOverlayFamily, overlayFamilyKey } from '../overlay/overlayFamily'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayContainerKey, overlayInlineKey } from '../overlay/overlayContainer'
import { dialogNestingKey } from '../overlay/dialogNesting'
import { useOverlay } from '../composables/useOverlay'

/**
 * Size: md is the default compact panel, wide fits a settings row beside a
 * long input, lg is an editor, xl is the settings shell.
 * Inline (a catalog host provides `overlayInlineKey`): the panel renders in flow at its
 * own size, with no scrim and no centering.
 * Nesting: a dialog opened from inside another stacks on it; the one beneath stays,
 * Escape and the scrim close only the top, and closing the one beneath takes the
 * stack with it.
 * Scrolling: a dialog's header, search and footer stay put; only its body scrolls.
 * The panel is a flex column capped at the host's height, so content declares one
 * root with `flex min-h-0 flex-col` and puts the body in a `ScrollArea`. Content
 * without that root simply scrolls as a whole.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  size?: 'md' | 'wide' | 'lg' | 'xl'
  label?: string
  /** Every dialog closes from its top-right corner; a flow that must finish can hide it. */
  hideClose?: boolean
  /** Stack on whatever dialog is open instead of replacing it, for a dialog mounted at the app root but opened from inside another. */
  stack?: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

// A dialog confined to a host container never blocks the page, so it is not exclusive.
const container = inject(overlayContainerKey, null)
const teleportTarget = computed(() => container?.value ?? 'body')
// Inline: the panel sits in flow at its own size, with no scrim, for a catalog specimen.
const inline = inject(overlayInlineKey, false)
const family = createOverlayFamily()
// Child menus share ownership, but the dialog surface is outside their click boundary.
provide(overlayFamilyKey, family)
const nested = inject(dialogNestingKey, false)
provide(dialogNestingKey, true)

const id = useOverlay(
  props.overlayStore,
  () => (container ? false : props.isOpen),
  () => {
    if (props.isOpen)
      emit('close')
  },
  nested || props.stack ? 'stacked' : 'exclusive',
)

onKeyStroke('Escape', (event) => {
  if (!props.isOpen || container || !props.overlayStore.isTop(id))
    return
  event.preventDefault()
  emit('close')
})
</script>

<template>
  <Teleport :to="teleportTarget" :disabled="inline">
    <!-- One transition for scrim and panel: a nested one never gets to leave, since the
         outer v-if unmounts the subtree. The panel's scale rides on the same stage classes. -->
    <Transition name="dialog" appear>
      <div
        v-if="isOpen"
        :class="inline ? 'dialog-scrim relative grid' : 'dialog-scrim fixed inset-0 z-50 grid place-items-center bg-black/40'"
        @click.self="!inline && emit('close')"
      >
        <div
          class="dialog-panel relative flex flex-col overflow-hidden rounded-xl bg-surface-dialog shadow-2xl"
          :class="[
            inline ? 'w-full' : 'max-h-[calc(100%-2rem)] w-[calc(100%-2rem)]',
            size === 'xl' ? 'max-w-5xl' : size === 'lg' ? 'max-w-3xl' : size === 'wide' ? 'max-w-xl' : 'max-w-md',
          ]"
          role="dialog"
          aria-modal="true"
          :aria-label="label"
        >
          <div v-if="!hideClose" class="absolute right-3 top-3 z-10">
            <IconButton
              :icon="X"
              variant="ghost"
              aria-label="Close"
              @click="emit('close')"
            />
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
