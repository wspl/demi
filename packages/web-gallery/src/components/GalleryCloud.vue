<script setup lang="ts">
import { ref, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import SettingsCloud from '@demicodes/web-ui/settings/SettingsCloud.vue'
import {
  destinationProblemText,
  type CloudIsolationSetting,
  type CloudNetworkSetting,
  type DestinationProblem,
  type Isolation,
  type IsolationRuntime,
  type PrivateNetworkAccess,
} from '@demicodes/web-ui/settings/cloud'
import { t } from '@demicodes/web-ui/settings/locales/text'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { slow } from '../fixtures/slow'

/** Which moment of Settings › Cloud the specimen starts at. */
export type CloudMoment = 'off' | 'entire' | 'only' | 'simple' | 'noGvisor' | 'loading' | 'loadFailed' | 'managerAway'

/**
 * Settings › Cloud with a stand-in server. It reads each destination the
 * way the server does and refuses what the server refuses: what is not an
 * IPv4 address or network, a port out of range, and the ranges no Cloud
 * ever reaches, such as 127.0.0.1 or 169.254.169.254. With the machine
 * manager away, a change is stored and the page says when it applies. A
 * server without gVisor offers Simple Isolation alone, and says why.
 */
const props = withDefaults(defineProps<{ moment?: CloudMoment }>(), { moment: 'only' })

const LISTED = ['10.20.0.0/16', '10.40.0.7:22']

/** What a server with root offers. */
const BOTH: IsolationRuntime[] = [
  { isolation: 'simple', unavailable: null, privateNetwork: false },
  { isolation: 'gvisor', unavailable: null, privateNetwork: true },
]

/** What a server that runs as an ordinary user offers. */
const SIMPLE_ONLY: IsolationRuntime[] = [
  { isolation: 'simple', unavailable: null, privateNetwork: false },
  { isolation: 'gvisor', unavailable: 'the machine manager does not run as root', privateNetwork: true },
]

const isolation = ref<CloudIsolationSetting | null>(null)
const network = ref<CloudNetworkSetting | null>(null)
const load = ref<'loading' | 'ready' | 'failed'>('ready')

function start(): void {
  switch (props.moment) {
    case 'loading':
      isolation.value = null
      network.value = null
      load.value = 'loading'
      return
    case 'loadFailed':
      isolation.value = null
      network.value = null
      load.value = 'failed'
      return
    case 'managerAway':
      isolation.value = { isolation: 'gvisor', runtimes: null }
      network.value = { access: 'only', destinations: [...LISTED] }
      load.value = 'ready'
      return
    case 'simple':
      isolation.value = { isolation: 'simple', runtimes: BOTH }
      network.value = { access: 'only', destinations: [...LISTED] }
      load.value = 'ready'
      return
    case 'noGvisor':
      isolation.value = { isolation: 'simple', runtimes: SIMPLE_ONLY }
      network.value = { access: 'off', destinations: [] }
      load.value = 'ready'
      return
    default:
      isolation.value = { isolation: 'gvisor', runtimes: BOTH }
      network.value = { access: props.moment, destinations: props.moment === 'only' ? [...LISTED] : [] }
      load.value = 'ready'
  }
}
watch(() => props.moment, start, { immediate: true })

async function retry(): Promise<void> {
  load.value = 'loading'
  await slow()
  isolation.value = { isolation: 'gvisor', runtimes: BOTH }
  network.value = { access: 'off', destinations: [] }
  load.value = 'ready'
}

/** Chooses `next`, as the server would, and says the Clouds move as they stop. */
async function setIsolation(next: Isolation): Promise<void> {
  if (!isolation.value) {
    return
  }
  await slow(300)
  isolation.value = { ...isolation.value, isolation: next }
  const message = props.moment === 'managerAway' ? t('cloud.pending') : t('cloud.moving', { isolation: t(`cloud.isolations.${next}`) })
  showToast({ title: t('cloud.title'), message, tone: 'neutral' })
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
    showToast({ title: t('cloud.title'), message: t('cloud.pending'), tone: 'neutral' })
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
  <SettingsCloud
    :isolation="isolation"
    :network="network"
    :load="load"
    :load-detail="load === 'failed' ? 'The stand-in server did not answer.' : null"
    :overlay-store="appOverlayStore"
    :on-retry="retry"
    :on-set-isolation="setIsolation"
    :on-set-access="setAccess"
    :save-destinations="saveDestinations"
  />
</template>
