<script setup lang="ts">
import { demoDeviceInstallation } from '../fixtures/device-installation'
import { computed } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import DevicePairingDialog from '@demicodes/web-ui/devices/DevicePairingDialog.vue'
import { pairingErrors, useDevicePairing, type PairingPhase } from '@demicodes/web-ui/devices/pairing'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import GalleryOverlayWell from './GalleryOverlayWell.vue'

const scenarios: {
  value: string;
  label: string;
  phase: PairingPhase
}[] = [
  { value: 'setup', label: 'Start runner', phase: { kind: 'setup' } },
  { value: 'code', label: 'Enter code', phase: { kind: 'code' } },
  { value: 'pairing', label: 'Pairing', phase: { kind: 'pairing' } },
  {
    value: 'error',
    label: 'Error',
    phase: {
      kind: 'code',
      error: pairingErrors.invalid_code
    }
  },
  {
    value: 'done',
    label: 'Connected',
    phase: {
      kind: 'done',
      device: { id: 'demo-device', name: 'zan-mbp' }
    }
  },
]
const { phase, submit, reset } = useDevicePairing(async () => {
  await new Promise((resolve) => window.setTimeout(resolve, 1000))
  return { ok: true, device: { id: 'demo-device', name: 'zan-mbp' } }
})
function select(value: string) {
  reset(scenarios.find((scenario) => scenario.value === value)!.phase)
}
const selected = computed(
  () => scenarios.find(
    (scenario) =>
      scenario.phase.kind === phase.value.kind &&
      (scenario.phase.kind !== 'code' ||
      phase.value.kind !== 'code' ||
      scenario.phase.error === phase.value.error)
  )?.value ?? 'setup'
)
</script>

<template>
  <div class="mb-3 flex flex-col gap-3">
    <p class="text-[13px] leading-5 text-fg-muted">Interactive preview: run through the buttons with any sample code, or select a state below. No machine is contacted. Installer commands are proposed examples, not published downloads.</p>
    <div class="flex flex-wrap"><Segmented
        :model-value="selected"
        :options="scenarios"
        @update:model-value="select"
      /></div>
  </div>
  <GalleryOverlayWell size="tall">
    <DevicePairingDialog
      :is-open="true"
      :overlay-store="appOverlayStore"
      :installation="demoDeviceInstallation"
      :phase="phase"
      @next="select('code')"
      @back="select('setup')"
      @submit="submit"
      @close="select('setup')"
    />
  </GalleryOverlayWell>
</template>
