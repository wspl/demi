<script setup lang="ts">
import { ref, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import SettingsCloudNetwork from '@demicodes/web-ui/settings/SettingsCloudNetwork.vue'
import {
  destinationProblemText,
  type CloudNetworkSetting,
  type DestinationProblem,
  type PrivateNetworkAccess,
} from '@demicodes/web-ui/settings/cloud-network'
import { t } from '@demicodes/web-ui/settings/locales/text'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { slow } from '../fixtures/slow'

/** Which moment of Settings › Cloud Network the specimen starts at. */
export type CloudNetworkMoment = 'off' | 'entire' | 'only' | 'loading' | 'loadFailed' | 'managerAway'

/**
 * Settings › Cloud Network with a stand-in server. It reads each
 * destination the way the server does and refuses what the server
 * refuses: what is not an IPv4 address or network, a port out of range,
 * and the ranges no Cloud ever reaches, such as 127.0.0.1 or
 * 169.254.169.254. With the machine manager away, a change is stored and
 * the page says when it applies.
 */
const props = withDefaults(defineProps<{ moment?: CloudNetworkMoment }>(), { moment: 'only' })

const LISTED = ['10.20.0.0/16', '10.40.0.7:22']

const network = ref<CloudNetworkSetting | null>(null)
const load = ref<'loading' | 'ready' | 'failed'>('ready')

function start(): void {
  switch (props.moment) {
    case 'loading':
      network.value = null
      load.value = 'loading'
      return
    case 'loadFailed':
      network.value = null
      load.value = 'failed'
      return
    case 'managerAway':
      network.value = { access: 'only', destinations: [...LISTED] }
      load.value = 'ready'
      return
    default:
      network.value = { access: props.moment, destinations: props.moment === 'only' ? [...LISTED] : [] }
      load.value = 'ready'
  }
}
watch(() => props.moment, start, { immediate: true })

async function retry(): Promise<void> {
  load.value = 'loading'
  await slow()
  network.value = { access: 'off', destinations: [] }
  load.value = 'ready'
}

/** Never reachable from a Cloud: this network, loopback, link-local, multicast and reserved. */
const NEVER: readonly (readonly [string, number])[] = [
  ['0.0.0.0', 8],
  ['127.0.0.0', 8],
  ['169.254.0.0', 16],
  ['224.0.0.0', 4],
  ['240.0.0.0', 4],
]

function numberOf(octets: readonly number[]): number {
  return (octets[0] << 24 | octets[1] << 16 | octets[2] << 8 | octets[3]) >>> 0
}

/** Why the stand-in server refuses `written`, or null for a destination it takes. */
function problemOf(written: string): DestinationProblem | null {
  const match = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?:\/(\d{1,2}))?(?::(.*))?$/.exec(written.trim())
  if (!match) {
    return 'address'
  }
  const octets = match.slice(1, 5).map(Number)
  const bits = match[5] === undefined ? 32 : Number(match[5])
  if (octets.some((octet) => octet > 255) || bits > 32) {
    return 'address'
  }
  if (match[6] !== undefined) {
    const port = Number(match[6])
    if (!/^\d+$/.test(match[6]) || port < 1 || port > 65535) {
      return 'port'
    }
  }
  const address = numberOf(octets)
  const overlaps = NEVER.some(([base, never]) => {
    const shared = Math.min(bits, never)
    const mask = shared === 0 ? 0 : (~0 << (32 - shared)) >>> 0
    return ((address & mask) >>> 0) === ((numberOf(base.split('.').map(Number)) & mask) >>> 0)
  })
  return overlaps ? 'neverReachable' : null
}

async function put(next: CloudNetworkSetting): Promise<SentenceText | null> {
  await slow(300)
  if (next.access === 'only') {
    for (const written of next.destinations) {
      const problem = problemOf(written)
      if (problem !== null) {
        return destinationProblemText(problem)
      }
    }
  }
  network.value = { access: next.access, destinations: next.destinations.map((written) => written.trim()) }
  if (props.moment === 'managerAway') {
    showToast({ title: t('cloudNetwork.title'), message: t('cloudNetwork.pending'), tone: 'neutral' })
  }
  return null
}

function setAccess(access: PrivateNetworkAccess): Promise<SentenceText | null> {
  return put({ access, destinations: network.value?.destinations ?? [] })
}

function saveDestinations(destinations: string[]): Promise<SentenceText | null> {
  return put({ access: 'only', destinations })
}
</script>

<template>
  <SettingsCloudNetwork
    :network="network"
    :load="load"
    :load-detail="load === 'failed' ? 'The stand-in server did not answer.' : null"
    :overlay-store="appOverlayStore"
    :on-retry="retry"
    :on-set-access="setAccess"
    :save-destinations="saveDestinations"
  />
</template>
