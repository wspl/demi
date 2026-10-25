<script setup lang="ts">
import { onUnmounted, ref, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import SettingsUpdates from '@demicodes/web-ui/settings/SettingsUpdates.vue'
import type { UpgradeChannel, ServerUpgrade } from '@demicodes/web-ui/settings/server-upgrade'
import { slow } from '../fixtures/slow'
import { productWould } from '../product-would'

/** Which moment of the server's updates the specimen starts at. */
export type UpdatesMoment = 'upToDate' | 'available' | 'downloading' | 'preparing' | 'failed' | 'checkFailed'

/**
 * Settings › Updates with a stand-in server that runs 0.1.5469:
 * Beta offers nothing newer and Canary offers 0.1.5500. Check Now looks
 * again after a moment, a channel looks for the newest of it, and Update Now
 * downloads 210 MB, prepares, and says the product would restart, where the
 * product shows every page the update's screen.
 */
const props = withDefaults(defineProps<{ moment?: UpdatesMoment }>(), { moment: 'available' })

const RUNNING = '0.1.5469'
const NEWER = '0.1.5500'
const NOTES = 'https://github.com/example/demi/releases/tag/v0.1.5500'
const BYTES = 210 * 1024 * 1024
/** The stand-in download's rate: 210 MB in about two minutes. */
const RATE = 1.75 * 1024 * 1024
const AT = '2026-10-25T09:00:00Z'

function startingAt(moment: UpdatesMoment): ServerUpgrade {
  const offered = { state: 'checked', at: AT, newer: NEWER, notesUrl: NOTES } as const
  switch (moment) {
    case 'upToDate':
      return { running: RUNNING, channel: 'beta', check: { state: 'checked', at: AT, newer: null }, run: { state: 'none' } }
    case 'available':
      return { running: RUNNING, channel: 'canary', check: offered, run: { state: 'none' } }
    case 'downloading':
      return { running: RUNNING, channel: 'canary', check: offered, run: { state: 'downloading', to: NEWER, bytesDone: BYTES * 0.42, bytes: BYTES, bytesPerSecond: RATE } }
    case 'preparing':
      return { running: RUNNING, channel: 'canary', check: offered, run: { state: 'preparing', to: NEWER } }
    case 'failed':
      return {
        running: RUNNING,
        channel: 'canary',
        check: offered,
        run: { state: 'failed', to: NEWER, reason: '0.1.5500 did not start: demi-backend.service failed' },
      }
    case 'checkFailed':
      return { running: RUNNING, channel: 'beta', check: { state: 'failed', at: AT, reason: 'GitHub answered 403 Forbidden' }, run: { state: 'none' } }
  }
}

const state = ref<ServerUpgrade>(startingAt(props.moment))
watch(() => props.moment, (moment) => (state.value = startingAt(moment)))
let stopped = false
onUnmounted(() => (stopped = true))

async function check(): Promise<void> {
  state.value = { ...state.value, check: { state: 'checking' } }
  await slow(800)
  // Canary's newest is NEWER, offered only to a server that runs an older one.
  const newer = state.value.channel === 'canary' && state.value.running !== NEWER ? NEWER : null
  state.value = { ...state.value, check: { state: 'checked', at: new Date().toISOString(), newer, notesUrl: newer ? NOTES : null } }
}

function setChannel(channel: UpgradeChannel): Promise<void> {
  state.value = { ...state.value, channel }
  return check()
}

async function update(version: string): Promise<void> {
  for (let done = 0; done <= BYTES && !stopped; done += BYTES / 8) {
    state.value = { ...state.value, run: { state: 'downloading', to: version, bytesDone: done, bytes: BYTES, bytesPerSecond: done === 0 ? 0 : RATE } }
    await slow(400)
  }
  state.value = { ...state.value, run: { state: 'preparing', to: version } }
  await slow(1200)
  productWould(`Restart in ${version}`)
  state.value = { ...startingAt('upToDate'), running: version, channel: state.value.channel }
}
</script>

<template>
  <SettingsUpdates
    :state="state"
    :overlay-store="appOverlayStore"
    :on-set-channel="setChannel"
    :on-check="check"
    :on-count-agents="async () => 2"
    :on-update="update"
  />
</template>
