<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ShareDialog from '@demicodes/web-ui/agent/ShareDialog.vue'
import { useShare, type ShareTarget } from '@demicodes/web-ui/agent/sharing'
import { useGalleryShares } from '../fixtures/sharing'
import GalleryDialogFrame from './GalleryDialogFrame.vue'

/**
 * The share dialog pinned on a share the conversation has, as the product
 * opens it on one that exists: its link, not copied. Copy Link copies, Update
 * Link and Stop Sharing work a beat and answer as the product's backend would;
 * Stop Sharing closes the dialog, and Open makes a new share and shows it.
 */
const props = defineProps<{ target: ShareTarget }>()

const { shares, service } = useGalleryShares([
  props.target.kind === 'conversation'
    ? { kind: 'conversation', token: 'Qm9iIHNoYXJlZCBhIGNvbnY' }
    : { kind: 'answer', token: 'QWxpY2UncyBhbnN3ZXI', blockId: props.target.blockId },
])
const sharing = useShare(() => service, () => shares.value)
void sharing.start(props.target)

/** Stop Sharing closes the specimen's frame once the share is removed. */
async function stop(close: () => void): Promise<void> {
  await sharing.stop()
  if (!sharing.isOpen.value) {
    close()
  }
}
</script>

<template>
  <GalleryDialogFrame v-slot="{ open, close }" @reopen="sharing.start(target)">
    <ShareDialog
      v-if="sharing.shown.value"
      :is-open="open && sharing.isOpen.value"
      :overlay-store="appOverlayStore"
      :kind="sharing.shown.value.target.kind"
      :url="sharing.shown.value.url"
      :token="sharing.shown.value.token"
      :copied="sharing.copied.value"
      :on-update="sharing.update"
      :on-stop="() => stop(close)"
      @copy="sharing.copy"
      @close="sharing.close(); close()"
    />
  </GalleryDialogFrame>
</template>
