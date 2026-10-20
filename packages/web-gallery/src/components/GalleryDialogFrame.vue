<script setup lang="ts">
import { provide, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { overlayContainerKey, overlayInlineKey } from '@demicodes/web-ui/overlay/overlayContainer'

/**
 * A dialog shown as a plain panel, in flow at its own size: no scrim, no
 * page-sized well. The frame is still the overlay container, so the dialog
 * never owns the page, and menus opened inside it float over the frame's edge.
 *
 * The frame holds whether its dialog is open, as the product's page does: the
 * dialog's own Cancel, Close and Done close it, and Open shows it again on its
 * pinned state.
 *
 * - `reopen` lets a specimen whose state moved put it back.
 * - `startsClosed` shows Open first, for a dialog that only a click opens, such
 *   as one that writes the clipboard.
 *
 * On a phone the frame is as wide as the product draws a dialog, the screen
 * less 1rem each side (`Dialog`'s 100% - 2rem), reaching past the catalog's
 * wider margins, so a specimen lays out as the product's dialog does.
 */
const props = defineProps<{ startsClosed?: boolean }>()
const emit = defineEmits<{ reopen: [] }>()
defineSlots<{ default(props: { open: boolean; close: () => void }): unknown }>()

const frame = ref<HTMLElement>()
provide(overlayContainerKey, frame)
provide(overlayInlineKey, true)

const open = ref(!props.startsClosed)

function close(): void {
  open.value = false
}

function reopen(): void {
  emit('reopen')
  open.value = true
}
</script>

<template>
  <div ref="frame" class="gallery-dialog-frame relative isolate">
    <slot :open="open" :close="close" />
    <Button v-if="!open" size="md" @click="reopen">Open</Button>
  </div>
</template>

<style scoped>
@media (max-width: 767px) {
  .gallery-dialog-frame {
    width: calc(100vw - 2rem);
    margin-inline: calc(50% - (100vw - 2rem) / 2);
  }
}
</style>
