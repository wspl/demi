<script setup lang="ts">
import { computed, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsPortForwards from '@demicodes/web-ui/settings/SettingsPortForwards.vue'
import { useProduct } from '../state/product'
import { portForwardPatchSchema } from '../api/generated/web-api'
import { useSettingsAddress } from './address'
import { usePortForwardSettings } from './port-forwards'

const product = useProduct()
const address = useSettingsAddress()
const settings = usePortForwardSettings()
/** The backend's limit on a forward's name, as its schema states it. */
const nameMaxLength = portForwardPatchSchema.shape.name.unwrap().maxLength ?? 64

/** Add Forward: the new forward's page opens once it exists. */
async function add(): Promise<void> {
  const id = await settings.add()
  if (id)
    await address.show('portForwards', id)
}

/** The forward whose page the address opens. */
const shown = computed(() => (address.section.value === 'portForwards' ? address.detail.value : null))
// A page of a forward that went, removed here or elsewhere, gives way to the list.
watch(
  [shown, () => product.snapshot],
  ([id, snapshot]) => {
    if (id && snapshot && !snapshot.portForwards.some((forward) => forward.id === id))
      void address.show('portForwards')
  },
)
</script>

<template>
  <SettingsPortForwards
    :forwards="settings.forwards"
    :hosts="settings.hosts"
    :shown="shown"
    :load="product.load"
    :name-max-length="nameMaxLength"
    :overlay-store="appOverlayStore"
    :on-add="add"
    :on-change="settings.change"
    :on-try-again="settings.tryAgain"
    :on-remove="settings.remove"
    @retry="product.reconnect"
    @show="address.show('portForwards', $event)"
  />
</template>
