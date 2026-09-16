<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsDevices from '@demicodes/web-ui/settings/SettingsDevices.vue'
import type { SettingsExpose } from '@demicodes/web-ui/settings/types'
import { demoDeviceInstallation } from '../fixtures/device-installation'
import { ahead } from '../fixtures/time'
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

function fixtures(): SettingsExpose[] {
  return [
    {
      id: 'k7x2m9qw4p3s6t8v0w2y4z6a8b',
      deviceId: 'mac',
      address: '127.0.0.1:5173',
      url: 'https://k7x2m9qw4p3s6t8v0w2y4z6a8b.expose.demi.example/',
      expiresAt: ahead(52 * 60_000),
    },
    {
      id: 'q9w8e7r6t5y4u3i2o1p0a1s2d3',
      deviceId: 'mac',
      address: '127.0.0.1:3000',
      url: 'https://q9w8e7r6t5y4u3i2o1p0a1s2d3.expose.demi.example/',
      expiresAt: ahead(45_000),
    },
    {
      id: 'm3n5p7r9t1v3w5x7y9z1a3c5e',
      deviceId: 'lab',
      address: '127.0.0.1:8080',
      url: 'https://m3n5p7r9t1v3w5x7y9z1a3c5e.expose.demi.example/',
      expiresAt: ahead(9 * 60_000),
    },
  ]
}

const live = ref(fixtures())
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
</template>
