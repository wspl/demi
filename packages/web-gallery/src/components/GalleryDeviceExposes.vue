<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import type { SettingsExpose } from '@demicodes/web-ui/settings/types'
import { demoDeviceInstallation } from '../fixtures/device-installation'
import { demoExposes } from '../fixtures/settings'
import type { CloudState } from '@demicodes/web-ui/cloud/types'

/**
 * The devices page pinned in its expose states: two devices with exposes (one
 * under a minute), the feature unavailable, and no exposes at all.
 */
const cloud = ref<CloudState>({
  state: 'running',
  phase: null,
  error: null,
  systemBytes: 16 * 1024 ** 3,
  homeBytes: 32 * 1024 ** 3,
})

const devices = [
  { id: 'mac', name: 'zan-mbp', online: true, seen: 'Now' },
  { id: 'lab', name: 'lab-workstation', online: true, seen: '2 minutes ago' },
]

const live = ref(demoExposes())
const empty = ref<SettingsExpose[]>([])
const pending = ref<string[]>([])

// Renew waits a beat so the loading state shows, then moves the expiry.
function renew(id: string) {
  if (pending.value.includes(id)) {
    return
  }
  pending.value.push(id)
  window.setTimeout(() => {
    const expose = live.value.find((entry) => entry.id === id)
    if (expose) {
      expose.expiresAt = new Date(Date.now() + 60 * 60_000).toISOString()
    }
    pending.value = pending.value.filter((entry) => entry !== id)
  }, 600)
}

function remove(id: string) {
  if (pending.value.includes(id)) {
    return
  }
  pending.value.push(id)
  window.setTimeout(() => {
    live.value = live.value.filter((entry) => entry.id !== id)
    pending.value = pending.value.filter((entry) => entry !== id)
  }, 600)
}

async function claimDevice() {
  await new Promise((resolve) => window.setTimeout(resolve, 900))
  return { ok: true as const, device: { id: 'new', name: 'host-new' } }
}
</script>

<template>
  <div class="flex flex-col gap-6">
    <div class="rounded-lg border border-line bg-surface">
      <SettingsDevices
        :devices="devices"
        :cloud="cloud"
        :overlay-store="appOverlayStore"
        :installation="demoDeviceInstallation"
        :claim-device="claimDevice"
        expose-domain="expose.demi.example"
        :exposes="live"
        :cloud-exposes="[]"
        :expose-pending-ids="pending"
        @renew-expose="renew"
        @remove-expose="remove"
      />
    </div>
    <div class="rounded-lg border border-line bg-surface">
      <SettingsDevices
        :devices="devices"
        :cloud="cloud"
        :overlay-store="appOverlayStore"
        :installation="demoDeviceInstallation"
        :claim-device="claimDevice"
        expose-domain="expose.demi.example"
        :exposes="empty"
        :cloud-exposes="[]"
      />
    </div>
    <div class="rounded-lg border border-line bg-surface">
      <SettingsDevices
        :devices="devices"
        :cloud="cloud"
        :overlay-store="appOverlayStore"
        :installation="demoDeviceInstallation"
        :claim-device="claimDevice"
        :expose-domain="null"
        :exposes="[]"
        :cloud-exposes="[]"
      />
    </div>
  </div>
</template>
