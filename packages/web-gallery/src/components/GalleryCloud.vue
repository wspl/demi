<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import SettingsCloud from '@demicodes/web-ui/settings/SettingsCloud.vue'
import {
  destinationProblemText,
  withPolicy,
  type CloudIsolationSetting,
  type CloudNetworkSetting,
  type CustomNetworkPolicy,
  type DestinationProblem,
  type Isolation,
  type IsolationDownload,
  type IsolationRuntime,
  type NetworkGroup,
  type NetworkPolicy,
} from '@demicodes/web-ui/settings/cloud'
import { t } from '@demicodes/web-ui/settings/locales/text'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'
import { slow } from '../fixtures/slow'

/** Which moment of Settings › Cloud the specimen starts at. */
export type CloudMoment =
  | 'internetOnly'
  | 'custom'
  | 'simple'
  | 'noGvisor'
  | 'needsDownload'
  | 'downloading'
  | 'downloadFailed'
  | 'loading'
  | 'loadFailed'
  | 'managerAway'

/**
 * Settings › Cloud with a stand-in server. It reads each entry the way the
 * server does and refuses what the server refuses: what is neither a domain
 * nor an IPv4 address or network, a port out of range, and the ranges no
 * Cloud ever reaches, such as 127.0.0.1 or 169.254.169.254. With the machine
 * manager away, a change is stored and the page says when it applies. A
 * server without gVisor offers Simple Isolation alone, and says why. One
 * that has never run gVisor downloads it when gVisor is chosen: the stand-in
 * receives its 160 MB in a few seconds, starts it, and runs it; choosing
 * Simple Isolation meanwhile cancels the download, and one that failed is
 * tried again by Try Again.
 */
const props = withDefaults(defineProps<{ moment?: CloudMoment }>(), { moment: 'custom' })

const LISTED = ['git.example.com', '*.registry.example.com', '10.20.0.0/16', '10.40.0.7:22']

/** The start of the list the server ships, enough for the row's count. */
const DEVELOPER_SERVICES = [
  'github.com', 'npmjs.org', 'pypi.org', 'crates.io', 'rubygems.org', 'golang.org', 'maven.org', 'nuget.org',
  'docker.io', 'ghcr.io', 'gitlab.com', 'bitbucket.org', 'yarnpkg.com', 'pythonhosted.org', 'debian.org',
  'ubuntu.com', 'alpinelinux.org', 'nodejs.org', 'rustup.rs', 'gradle.org', 'packagist.org', 'pub.dev',
  'hex.pm', 'cocoapods.org', 'swift.org', 'quay.io', 'gcr.io', 'k8s.io', 'apache.org', 'cpan.org',
  'metacpan.org', 'haskell.org', 'githubusercontent.com', 'ruby-lang.org', 'dot.net', 'mcr.microsoft.com',
  'packages.microsoft.com', 'archlinux.org', 'fedoraproject.org', 'centos.org', 'launchpad.net',
  'ppa.launchpad.net', 'anaconda.com', 'continuum.io', 'pypa.io', 'pypi.python.org', 'goproxy.io',
  'pkg.go.dev', 'java.com', 'java.net', 'spring.io', 'eclipse.org', 'sourceforge.net', 'hashicorp.com',
  'docker.com', 'json.schemastore.org', 'json-schema.org', 'jcenter.bintray.com', 'packagecloud.io',
  'rvm.io', 'rubyonrails.org', 'rubyforge.org', 'bower.io', 'npmjs.com', 'apt.llvm.org', 'azure.com',
  'visualstudio.com', 'dotnet.microsoft.com', 'oracle.com', 'microsoft.com', 'google.com',
]

/** Custom as a server that upgraded from a list of private addresses holds it. */
function custom(allowed: string[]): CustomNetworkPolicy {
  return { internet: true, privateNetwork: false, developerServices: false, allowed }
}

function setting(policy: NetworkPolicy, allowed: string[] = [...LISTED]): CloudNetworkSetting {
  return { policy, custom: custom(allowed), developerServices: DEVELOPER_SERVICES }
}

/** What a server with root offers. */
const BOTH: IsolationRuntime[] = [
  { isolation: 'simple', unavailable: null, networkPolicy: false, download: null },
  { isolation: 'gvisor', unavailable: null, networkPolicy: true, download: null },
]

/** What a server that runs as an ordinary user offers. */
const SIMPLE_ONLY: IsolationRuntime[] = [
  { isolation: 'simple', unavailable: null, networkPolicy: false, download: null },
  { isolation: 'gvisor', unavailable: 'the machine manager does not run as root', networkPolicy: true, download: null },
]

/** What a server with root offers before it has ever run gVisor, with gVisor's download at `download`. */
function beforeGvisor(download: IsolationDownload): IsolationRuntime[] {
  return [
    { isolation: 'simple', unavailable: null, networkPolicy: false, download: null },
    { isolation: 'gvisor', unavailable: null, networkPolicy: true, download },
  ]
}

/** gVisor's release for the server's architecture, as the stand-in receives it. */
const RELEASE_BYTES = 160_000_000
const FAILURE = 'https://storage.example.com/gvisor/release/gvisor.tar.bz2: the connection was reset'

const isolation = ref<CloudIsolationSetting | null>(null)
const network = ref<CloudNetworkSetting | null>(null)
const load = ref<'loading' | 'ready' | 'failed'>('ready')

/** Stops the stand-in's download under way, or its start. */
let stopping: (() => void) | null = null

function stopDownload(): void {
  stopping?.()
  stopping = null
}
onBeforeUnmount(stopDownload)

/** Sets gVisor's download to `download`, keeping the choice. */
function setDownload(download: IsolationDownload): void {
  if (isolation.value) {
    isolation.value = { ...isolation.value, runtimes: beforeGvisor(download) }
  }
}

/** Receives gVisor's release a piece at a time, starts it, then runs it, as the server does. */
function download(): void {
  stopDownload()
  let received = 0
  setDownload({ state: 'downloading', received, total: RELEASE_BYTES })
  const receiving = setInterval(() => {
    received = Math.min(RELEASE_BYTES, received + RELEASE_BYTES / 10)
    if (received < RELEASE_BYTES) {
      setDownload({ state: 'downloading', received, total: RELEASE_BYTES })
      return
    }
    stopDownload()
    setDownload({ state: 'starting' })
    const starting = setTimeout(() => {
      stopping = null
      if (isolation.value) {
        isolation.value = { ...isolation.value, runtimes: BOTH }
      }
    }, 2000)
    stopping = () => clearTimeout(starting)
  }, 400)
  stopping = () => clearInterval(receiving)
}

function start(): void {
  stopDownload()
  switch (props.moment) {
    case 'needsDownload':
      isolation.value = { isolation: 'simple', runtimes: beforeGvisor({ state: 'waiting' }) }
      network.value = setting('internetOnly', [])
      load.value = 'ready'
      return
    case 'downloading':
      isolation.value = { isolation: 'gvisor', runtimes: beforeGvisor({ state: 'waiting' }) }
      network.value = setting('internetOnly', [])
      load.value = 'ready'
      download()
      return
    case 'downloadFailed':
      isolation.value = { isolation: 'gvisor', runtimes: beforeGvisor({ state: 'failed', message: FAILURE }) }
      network.value = setting('internetOnly', [])
      load.value = 'ready'
      return
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
      network.value = setting('custom')
      load.value = 'ready'
      return
    case 'simple':
      isolation.value = { isolation: 'simple', runtimes: BOTH }
      network.value = setting('custom')
      load.value = 'ready'
      return
    case 'noGvisor':
      isolation.value = { isolation: 'simple', runtimes: SIMPLE_ONLY }
      network.value = setting('internetOnly', [])
      load.value = 'ready'
      return
    default:
      isolation.value = { isolation: 'gvisor', runtimes: BOTH }
      network.value = setting(props.moment, props.moment === 'custom' ? [...LISTED] : [])
      load.value = 'ready'
  }
}
watch(() => props.moment, start, { immediate: true })

async function retry(): Promise<void> {
  stopDownload()
  load.value = 'loading'
  await slow()
  isolation.value = { isolation: 'gvisor', runtimes: BOTH }
  network.value = setting('internetOnly', [])
  load.value = 'ready'
}

/**
 * Chooses `next`, as the server would, and says the Clouds move as they
 * stop: after the download, when `next` needs one, which choosing the other
 * cancels. Leaving an isolation that awaits its download moves nothing.
 */
async function setIsolation(next: Isolation): Promise<void> {
  if (!isolation.value) {
    return
  }
  await slow(300)
  const runtime = isolation.value.runtimes?.find((found) => found.isolation === next)
  const downloads = runtime?.download?.state === 'waiting' || runtime?.download?.state === 'failed'
  const left = isolation.value.runtimes?.find((found) => found.isolation === isolation.value?.isolation)?.download
  isolation.value = { ...isolation.value, isolation: next }
  if (next === 'simple' && stopping !== null && isolation.value.runtimes?.some((found) => found.download?.state === 'downloading')) {
    stopDownload()
    setDownload({ state: 'waiting' })
  }
  if (downloads) {
    download()
  }
  const message =
    props.moment === 'managerAway'
      ? t('cloud.pending')
      : left
        ? null
        : downloads
          ? t('cloud.download.moving')
          : t('cloud.moving', { isolation: t(`cloud.isolations.${next}`) })
  if (message) {
    showToast({ title: t('cloud.title'), message, tone: 'neutral' })
  }
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

/** A domain, `git.example.com` or `*.example.com`, with an optional port. */
const DOMAIN = /^(?:\*\.)?(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z][a-z0-9-]*[a-z0-9](?::(.*))?$/i

/** Why the stand-in server refuses `written`, or null for an entry it takes. */
function problemOf(written: string): DestinationProblem | null {
  const domain = DOMAIN.exec(written.trim())
  if (domain) {
    return domain[1] === undefined || /^\d+$/.test(domain[1]) && Number(domain[1]) >= 1 && Number(domain[1]) <= 65535 ? null : 'port'
  }
  const match = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?:\/(\d{1,2}))?(?::(.*))?$/.exec(written.trim())
  if (!match) {
    return 'entry'
  }
  const octets = match.slice(1, 5).map(Number)
  const bits = match[5] === undefined ? 32 : Number(match[5])
  if (octets.some((octet) => octet > 255) || bits > 32) {
    return 'entry'
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
  for (const written of next.custom.allowed) {
    const problem = problemOf(written)
    if (problem !== null) {
      return destinationProblemText(problem)
    }
  }
  network.value = { ...next, custom: { ...next.custom, allowed: next.custom.allowed.map((written) => written.trim()) } }
  if (props.moment === 'managerAway') {
    showToast({ title: t('cloud.title'), message: t('cloud.pending'), tone: 'neutral' })
  }
  return null
}

function setPolicy(policy: NetworkPolicy): Promise<SentenceText | null> | undefined {
  return network.value ? put(withPolicy(network.value, policy)) : undefined
}

function setGroup(group: NetworkGroup, on: boolean): Promise<SentenceText | null> | undefined {
  const current = network.value
  return current ? put({ ...current, custom: { ...current.custom, [group]: on } }) : undefined
}

function saveAllowed(allowed: string[]): Promise<SentenceText | null> {
  const current = network.value ?? setting('custom', [])
  return put({ ...current, custom: { ...current.custom, allowed } })
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
    :on-set-policy="setPolicy"
    :on-set-group="setGroup"
    :save-allowed="saveAllowed"
  />
</template>
