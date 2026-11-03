<script setup lang="ts">
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ProviderShareDialog from '@demicodes/web-ui/settings/ProviderShareDialog.vue'
import { useProviderShare } from '@demicodes/web-ui/settings/provider-sharing'
import { galleryShareKeys, useGalleryShareKeys } from '../fixtures/provider-sharing'
import GalleryDialogFrame from './GalleryDialogFrame.vue'

/**
 * Share Provider as the product opens it, in one of three states:
 *
 * - `keys`: on a provider with keys, one used and one not, and no key shown.
 * - `new`: from Open, which stands for Share since only a click may write the
 *   clipboard, on a provider without keys, making its first with Creating
 *   key… and then showing it, copied.
 * - `failing`: as `new`, but making the key fails first, with Try Again.
 *
 * New Key makes another and Revoke removes one, a beat later, as the product's
 * backend would; revoking the used key asks first.
 */
const props = defineProps<{ state: 'keys' | 'new' | 'failing' }>()

const { service } = useGalleryShareKeys(props.state === 'keys' ? galleryShareKeys() : [], { failFirst: props.state === 'failing' })
const sharing = useProviderShare(() => service)
const provider = { id: 'anthropic', name: 'Anthropic' }
if (props.state === 'keys') {
  sharing.start(provider)
}
</script>

<template>
  <GalleryDialogFrame v-slot="{ open, close }" :starts-closed="state !== 'keys'" @reopen="sharing.start(provider)">
    <ProviderShareDialog
      :is-open="open && sharing.isOpen.value"
      :overlay-store="appOverlayStore"
      :name="provider.name"
      :keys="sharing.keys.value"
      :made="sharing.made.value"
      :copied="sharing.copied.value"
      :on-revoke="sharing.revoke"
      @copy="sharing.copy"
      @make="sharing.make"
      @reload="sharing.reload"
      @close="sharing.close(); close()"
    />
  </GalleryDialogFrame>
</template>
