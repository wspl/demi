<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ShareDialog from '@demicodes/web-ui/agent/ShareDialog.vue'
import { useShare, type ShareTarget } from '@demicodes/web-ui/agent/sharing'
import { useGalleryShares } from '../fixtures/sharing'
import GalleryDialogFrame from './GalleryDialogFrame.vue'

/**
 * The share dialog as the product opens it, in one of three states:
 *
 * - `existing`: on a share the conversation has, with its link, not copied.
 * - `new`: from Open, which stands for Share since only a click may write the
 *   clipboard, on a new share, making its link with Creating link… and then
 *   showing it, copied.
 * - `failing`: on one whose making fails first, with Try Again.
 *
 * Copy Link copies. Update Link and Stop Sharing work a beat and answer as the
 * product's backend would. Stop Sharing closes the dialog, and Open makes a new
 * share and shows it.
 */
const props = defineProps<{ target: ShareTarget; state: 'existing' | 'new' | 'failing' }>()

const { shares, service } = useGalleryShares(
  props.state !== 'existing'
    ? []
    : props.target.kind === 'conversation'
      ? [{ kind: 'conversation', token: 'Qm9iIHNoYXJlZCBhIGNvbnY' }]
      : [{ kind: 'answer', token: 'QWxpY2UncyBhbnN3ZXI', blockId: props.target.blockId }],
  { failFirst: props.state === 'failing' },
)
const sharing = useShare(() => service, () => shares.value)
if (props.state === 'existing') {
  sharing.start(props.target)
}

/** Stop Sharing closes the specimen's frame once the share is removed. */
async function stop(close: () => void): Promise<void> {
  await sharing.stop()
  if (!sharing.isOpen.value) {
    close()
  }
}
</script>

<template>
  <GalleryDialogFrame v-slot="{ open, close }" :starts-closed="state !== 'existing'" @reopen="sharing.start(target)">
    <ShareDialog
      v-if="sharing.shown.value"
      :is-open="open && sharing.isOpen.value"
      :overlay-store="appOverlayStore"
      :kind="sharing.shown.value.target.kind"
      :phase="sharing.shown.value"
      :copied="sharing.copied.value"
      :on-update="sharing.update"
      :on-stop="() => stop(close)"
      @copy="sharing.copy"
      @retry="sharing.retry"
      @close="sharing.close(); close()"
    />
  </GalleryDialogFrame>
</template>
