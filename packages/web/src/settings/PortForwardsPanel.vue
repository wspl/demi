<script setup lang="ts">
import { computed, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsPortForwards from '@demicodes/web-ui/settings/SettingsPortForwards.vue'
import { useProduct } from '../state/product'
import { useSettingsAddress } from './address'
import { usePortForwardSettings } from './port-forwards'

const product = useProduct()
const address = useSettingsAddress()
const settings = usePortForwardSettings()

/** The forward whose page the address opens. */
const shown = computed(() => (address.section.value === 'portForwards' ? address.detail.value : null))
// A page of a forward that went, removed here or elsewhere, gives way to the list.
watch(
  [shown, () => product.snapshot],
  ([id, snapshot]) => {
    if (id && snapshot?.portForwards && !snapshot.portForwards.some((forward) => forward.id === id))
      void address.show('portForwards')
  },
)
</script>

<template>
  <SettingsPortForwards
    :forwards="settings.forwards"
    :hosts="settings.hosts"
    :shown="shown"
    :load="product.listLoad('port_forwards')"
    :overlay-store="appOverlayStore"
    :on-add="settings.add"
    :on-change="settings.change"
    :on-try-again="settings.tryAgain"
    :on-remove="settings.remove"
    @retry="product.retryList('port_forwards')"
    @show="address.show('portForwards', $event)"
  />
</template>
