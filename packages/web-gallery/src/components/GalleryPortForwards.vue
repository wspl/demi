<script setup lang="ts">
import { ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { coded } from '@demicodes/web-ui/infra/errors'
import SettingsPortForwards from '@demicodes/web-ui/settings/SettingsPortForwards.vue'
import type { ForwardHost, PortForwardChange, SettingsPortForward } from '@demicodes/web-ui/settings/port-forwards'
import { GALLERY_MARKS, shownAmong } from '../fixtures/device-marks'
import { slow } from '../fixtures/slow'

/**
 * Settings › Port Forwarding with a forward in each state of the design's
 * table. Every control acts on the specimen's own forwards as the product's
 * do:
 *
 * - Add Forward makes one and opens it.
 * - A taken name is refused.
 * - Try Again finds the port free.
 * - Remove Forward drops it.
 */
const hosts = ref<ForwardHost[]>(shownAmong([
  { id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null },
  { id: 'mac', name: 'alex-mbp', kind: 'device', state: 'online', mark: GALLERY_MARKS.mac },
  { id: 'build', name: 'build-01', kind: 'device', state: 'offline', mark: GALLERY_MARKS.ubuntu },
]))

const forwards = ref<SettingsPortForward[]>([
  { id: 'f1', name: 'ollama', listenDevice: 'cloud', listenPort: 11434, connectDevice: 'mac', connectAddress: 'localhost:11434', enabled: true, state: { kind: 'listening', direct: 0, relay: 2, reason: { kind: 'unreachable' } } },
  { id: 'f2', name: 'vite', listenDevice: 'mac', listenPort: 5173, connectDevice: 'cloud', connectAddress: 'localhost:5173', enabled: true, state: { kind: 'listening', direct: 0, relay: 0 } },
  { id: 'f3', name: 'postgres', listenDevice: 'cloud', listenPort: 5432, connectDevice: 'mac', connectAddress: '127.0.0.1:5432', enabled: true, state: { kind: 'portInUse' } },
  { id: 'f4', name: 'builds', listenDevice: 'mac', listenPort: 9000, connectDevice: 'build', connectAddress: 'localhost:9000', enabled: true, state: { kind: 'listening', direct: 0, relay: 0 } },
  { id: 'f5', name: 'grafana', listenDevice: 'mac', listenPort: 3000, connectDevice: 'cloud', connectAddress: 'localhost:3000', enabled: false, state: null },
])
const shown = ref<string | null>(null)
let made = 0

/** The first free name, *Forward*, *Forward (2)*, and port, from 8080, as the backend picks them. */
async function add(): Promise<void> {
  await slow()
  const names = new Set(forwards.value.map((forward) => forward.name))
  let name = 'Forward'
  for (let n = 2; names.has(name); n++)
    name = `Forward (${n})`
  const ports = new Set(forwards.value.filter((forward) => forward.listenDevice === 'cloud').map((forward) => forward.listenPort))
  let port = 8080
  while (ports.has(port))
    port++
  made += 1
  const id = `new${made}`
  forwards.value = [...forwards.value, { id, name, listenDevice: 'cloud', listenPort: port, connectDevice: null, connectAddress: 'localhost:11434', enabled: true, state: { kind: 'listening', direct: 0, relay: 0 } }]
  shown.value = id
}

/** A change lands as the backend's answer does; a name another forward has is refused in the field. */
async function change(id: string, change: PortForwardChange): Promise<void> {
  await slow()
  if (change.name !== undefined && forwards.value.some((forward) => forward.id !== id && forward.name === change.name))
    throw coded(new Error(`A forward named ${change.name} already exists`), 'port_forward_name_taken', { name: change.name })
  forwards.value = forwards.value.map((forward) => {
    if (forward.id !== id)
      return forward
    const next = { ...forward, ...change }
    // Listening again on another port or Host, or turned on, it starts idle.
    const relisten = change.listenPort !== undefined || change.listenDevice !== undefined || change.enabled === true
    return relisten ? { ...next, state: { kind: 'listening', direct: 0, relay: 0 } } : change.enabled === false ? { ...next, state: null } : next
  })
}

/** Try Again: the port is free now. */
async function tryAgain(id: string): Promise<void> {
  await slow()
  forwards.value = forwards.value.map((forward) => forward.id === id ? { ...forward, state: { kind: 'listening', direct: 0, relay: 0 } } : forward)
}

async function remove(id: string): Promise<void> {
  await slow()
  forwards.value = forwards.value.filter((forward) => forward.id !== id)
  shown.value = null
}
</script>

<template>
  <SettingsPortForwards
    :forwards="forwards"
    :hosts="hosts"
    :shown="shown"
    load="ready"
    :name-max-length="64"
    :overlay-store="appOverlayStore"
    :on-add="add"
    :on-change="change"
    :on-try-again="tryAgain"
    :on-remove="remove"
    @retry="() => {}"
    @show="shown = $event"
  />
</template>
