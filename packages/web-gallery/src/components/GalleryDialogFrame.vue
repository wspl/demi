<script setup lang="ts">
import { provide, ref } from 'vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { overlayContainerKey, overlayInlineKey } from '@demicodes/web-ui/overlay/overlayContainer'

/**
 * A dialog shown as a plain panel, in flow at its own size: no scrim, no page-sized
 * well. The frame is still the overlay container, so the dialog never owns the page,
 * and menus opened inside it float over the frame's edge.
 *
 * The frame holds whether its dialog is open, as the product's page does: the
 * dialog's own Cancel, Close and Done close it, and Open shows it again on its
 * pinned state. `reopen` lets a specimen whose state moved put it back.
 */
const emit = defineEmits<{ reopen: [] }>()
defineSlots<{ default(props: { open: boolean; close: () => void }): unknown }>()

const frame = ref<HTMLElement>()
provide(overlayContainerKey, frame)
provide(overlayInlineKey, true)

const open = ref(true)

function close(): void {
  open.value = false
}

function reopen(): void {
  emit('reopen')
  open.value = true
}
</script>

<template>
  <div ref="frame" class="relative isolate">
    <slot :open="open" :close="close" />
    <Button v-if="!open" size="md" @click="reopen">Open</Button>
  </div>
</template>
