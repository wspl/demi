import { onBeforeUnmount, ref, watch } from 'vue'
import type { DeviceRoute, DirectAttempt, DirectStatus, PathFigures } from '@demicodes/web-ui/devices/direct'
import type { SettingsDevice } from '@demicodes/web-ui/settings/types'
import { demoDeviceReport, demoDeviceStart } from './device-installation'
import { ago } from './time'

/**
 * The paths a demo device's attempts take, one per state its page shows
 * (`direct-channel.md` § What the user sees), and the devices of the
 * Devices specimens, whose controls act on them as the product's do: Try
 * Again runs an attempt that ends as the device's path says, Measure and
 * the page showing measure the paths again, the route changes how it is
 * reached, and Rename… and Revoke… change the list.
 */
export type DirectScenario =
  | 'connected'
  | 'thisComputer'
  | 'internet'
  | 'slower'
  | 'serverOnly'
  | 'blocked'
  | 'unreachable'
  | 'browserNetwork'
  | 'deviceNetwork'
  | 'busy'
  | 'dropped'
  | 'notOffered'

export const DIRECT_SCENARIOS: readonly { value: DirectScenario; label: string }[] = [
  { value: 'connected', label: 'LAN' },
  { value: 'thisComputer', label: 'This Computer' },
  { value: 'internet', label: 'P2P' },
  { value: 'slower', label: 'Slower' },
  { value: 'serverOnly', label: 'Relay Only' },
  { value: 'blocked', label: 'Blocked' },
  { value: 'unreachable', label: 'Not Reachable' },
  { value: 'browserNetwork', label: 'This Network' },
  { value: 'deviceNetwork', label: 'Device’s Network' },
  { value: 'busy', label: 'Busy' },
  { value: 'dropped', label: 'Dropped' },
  { value: 'notOffered', label: 'Not Offered' },
]

const minutes = (count: number) => count * 60_000

/** The relay's figures to a server about 230 ms away, as the design measured. */
const RELAY: PathFigures = { latencyMs: 480, loss: null }
/** Each standing peer's path: its address in use and what a measurement finds on it. */
const PEER_PATHS: Partial<Record<DirectScenario, { address: string; figures: PathFigures }>> = {
  connected: { address: '192.168.1.20', figures: { latencyMs: 1.8, loss: 0 } },
  thisComputer: { address: '127.0.0.1', figures: { latencyMs: 0.3, loss: 0 } },
  internet: { address: '203.0.113.9', figures: { latencyMs: 38, loss: 0 } },
  // A congested link, slower than the relay and losing two probes in twenty.
  slower: { address: '203.0.113.9', figures: { latencyMs: 620, loss: 0.1 } },
}

/** How long a demo measurement takes: its 20 probes, 100 ms apart, and the last answer. */
const MEASURE_MS = 2_400

/** Whether an attempt of `scenario` connects. */
const connects = (scenario: DirectScenario) => scenario in PEER_PATHS || scenario === 'dropped'

/** What an attempt of `scenario` that began at `startedAt` saw. */
export function demoAttempt(scenario: DirectScenario, startedAt: number): DirectAttempt {
  const browserPublic = scenario === 'browserNetwork' || scenario === 'notOffered' ? [] : ['198.51.100.24']
  const devicePublic = scenario === 'deviceNetwork' || scenario === 'notOffered' ? [] : ['203.0.113.9']
  const connected = connects(scenario)
  return {
    startedAt: new Date(startedAt).toISOString(),
    durationMs: connected ? 46 : scenario === 'busy' ? 120 : 10_000,
    outcome: scenario === 'busy' ? 'busy' : scenario === 'dropped' ? 'dropped' : connected ? 'connected' : 'failed',
    stage: scenario === 'busy' ? null : connected ? 'connected' : scenario === 'blocked' ? 'permission' : 'checking',
    browser: { local: ['7c1e4a52-9b0d-4c1e-8e3e-1a2b3c4d5e6f.local'], public: browserPublic },
    device: { local: ['192.168.1.20', '127.0.0.1'], public: devicePublic },
    pairs: { tried: connected ? 2 : 6, answered: connected ? 2 : 0 },
    inUse: connected ? { address: PEER_PATHS[scenario]?.address ?? '192.168.1.20', port: 61204 } : null,
    permission: scenario === 'blocked' ? 'denied' : 'granted',
  }
}

/** How a device of `scenario` is reached, its last attempt a few minutes ago. */
export function demoDirect(scenario: DirectScenario): DirectStatus {
  const now = Date.now()
  const path = PEER_PATHS[scenario]
  const peer = path !== undefined
  return {
    route: scenario === 'serverOnly' ? 'server' : 'automatic',
    crossing: scenario !== 'notOffered',
    permission: scenario === 'blocked' ? 'denied' : 'granted',
    peer,
    chosen: peer && scenario !== 'slower',
    trying: false,
    attempt: scenario === 'serverOnly' ? null : demoAttempt(scenario, now - minutes(3)),
    nextAt: peer || scenario === 'serverOnly' || scenario === 'blocked' ? null : new Date(now + minutes(7)).toISOString(),
    figures: { direct: path?.figures ?? null, relay: RELAY },
    measuring: false,
  }
}

/** What a measurement of `scenario`'s paths finds, each latency a little off its usual figure as a real one's is. */
function measuredFigures(scenario: DirectScenario, peer: boolean): DirectStatus['figures'] {
  const vary = (figures: PathFigures): PathFigures => ({ ...figures, latencyMs: figures.latencyMs * (0.9 + Math.random() * 0.2) })
  const direct = peer ? PEER_PATHS[scenario]?.figures : undefined
  return { direct: direct ? vary(direct) : null, relay: vary(RELAY) }
}

/** A device of a specimen, and how its attempts end. */
export interface GalleryDevice {
  device: SettingsDevice
  scenario: DirectScenario
}

const day = minutes(24 * 60)

/**
 * A demo device, paired `paired` ago, reached as `scenario` says once it is
 * online; while it is offline this page has made no attempt and measured
 * nothing.
 */
function demoDevice(
  device: Pick<SettingsDevice, 'id' | 'name' | 'state' | 'seen' | 'start'> & { os: SettingsDevice['os']; runnerVersion: string | null },
  scenario: DirectScenario,
  paired: number,
): GalleryDevice {
  const reached = demoDirect(scenario)
  const direct =
    device.state === 'offline'
      ? { ...reached, peer: false, chosen: false, attempt: null, nextAt: null, figures: { direct: null, relay: null } }
      : reached
  return { device: { ...device, pairedAt: ago(paired), direct }, scenario }
}

/** The release the demo server's runners follow. */
export const DEMO_RUNNER_RELEASE = '9c1f…release'

/** The paired devices of the Devices specimens. */
export function galleryDevices(): GalleryDevice[] {
  const current = (system: 'macos' | 'linux' | 'windows') => demoDeviceReport(system, DEMO_RUNNER_RELEASE)
  return [
    demoDevice({ id: 'mac', name: 'zan-mbp', state: 'online', seen: ago(0), ...current('macos') }, 'connected', 40 * day),
    demoDevice(
      { id: 'build', name: 'build-01', state: 'offline', seen: ago(3 * day), start: demoDeviceStart('linux'), ...current('linux') },
      'unreachable',
      90 * day,
    ),
    demoDevice(
      { id: 'studio', name: 'studio-pc', state: 'online', seen: ago(0), ...demoDeviceReport('windows', 'an-older-release') },
      'unreachable',
      12 * day,
    ),
    demoDevice(
      { id: 'lab', name: 'lab-workstation-with-a-long-hostname', state: 'updating', seen: ago(minutes(2)), ...current('linux') },
      'notOffered',
      2 * day,
    ),
  ]
}

/**
 * The state of a Devices specimen: its devices, the page shown, and what
 * each control does to them. An attempt takes a moment.
 */
export function useGalleryDevices(initial: () => GalleryDevice[]) {
  const made = initial()
  const devices = ref<SettingsDevice[]>(made.map((entry) => entry.device))
  const shown = ref<string | null>(null)
  /** How each device's attempts end. */
  const scenarios = ref<Record<string, DirectScenario>>(
    Object.fromEntries(made.map((entry) => [entry.device.id, entry.scenario])),
  )
  const timers = new Set<number>()
  onBeforeUnmount(() => {
    for (const timer of timers)
      window.clearTimeout(timer)
  })
  /** Runs `then` after `ms`, unless the specimen goes first. */
  const later = (ms: number, then: () => void) => {
    const timer = window.setTimeout(() => {
      timers.delete(timer)
      then()
    }, ms)
    timers.add(timer)
  }

  const find = (id: string) => devices.value.find((device) => device.id === id)
  const scenarioOf = (device: SettingsDevice): DirectScenario => scenarios.value[device.id] ?? 'connected'

  /** Sets how device `id` is reached, as if its paths had gone that way. */
  function setScenario(id: string, scenario: DirectScenario) {
    scenarios.value[id] = scenario
    const device = find(id)
    if (device)
      device.direct = demoDirect(scenario)
  }

  /**
   * Measures device `id`'s paths, as the product does when a peer
   * connects, the page shows or Measure is selected; one asked for while
   * one runs joins it, and the figures shown stay until it ends.
   */
  function measure(id: string) {
    const device = find(id)
    if (!device || device.state !== 'online' || device.direct.measuring)
      return
    device.direct.measuring = true
    later(MEASURE_MS, () => {
      device.direct.figures = measuredFigures(scenarioOf(device), device.direct.peer)
      device.direct.measuring = false
    })
  }
  // A device's page measures its paths each time it shows.
  watch(shown, (id) => {
    if (id)
      measure(id)
  }, { immediate: true })

  function tryNow(id: string) {
    const device = find(id)
    if (!device || device.direct.trying)
      return
    device.direct.trying = true
    device.direct.nextAt = null
    const startedAt = Date.now()
    later(900, () => {
      // An attempt that ends as the device's path says; one tried again
      // after its route allowed it, or after its channel dropped, connects.
      const scenario = scenarioOf(device)
      const ends = scenario === 'serverOnly' || scenario === 'dropped' ? 'connected' : scenario
      const status = demoDirect(ends)
      // The figures shown stay until a measurement replaces them; a new peer is measured at once.
      const { figures, measuring } = device.direct
      device.direct = { ...status, route: device.direct.route, attempt: demoAttempt(ends, startedAt), figures, measuring }
      if (status.peer)
        measure(device.id)
    })
  }

  function setRoute(id: string, route: DeviceRoute) {
    const device = find(id)
    if (!device)
      return
    device.direct.route = route
    if (route === 'server') {
      device.direct.peer = false
      device.direct.chosen = false
      device.direct.trying = false
      device.direct.nextAt = null
      device.direct.figures = { ...device.direct.figures, direct: null }
      return
    }
    if (device.direct.peer) {
      // Prefer P2P uses a standing peer whatever Automatic found.
      device.direct.chosen = route === 'direct' || scenarioOf(device) !== 'slower'
      return
    }
    tryNow(id)
  }

  function rename(id: string, name: string) {
    const device = find(id)
    if (device)
      device.name = name
  }

  /** A revoked device leaves the list, and its page gives way to the list. */
  function revoke(id: string) {
    devices.value = devices.value.filter((device) => device.id !== id)
    if (shown.value === id)
      shown.value = null
  }

  /** A device the pairing dialog paired joins the list, online and tried at once. */
  async function claim(_code: string) {
    await new Promise((resolve) => window.setTimeout(resolve, 900))
    const device: SettingsDevice = {
      id: `device-${Date.now()}`,
      name: `host-${devices.value.length + 1}`,
      state: 'online',
      seen: new Date().toISOString(),
      pairedAt: new Date().toISOString(),
      ...demoDeviceReport('macos', DEMO_RUNNER_RELEASE),
      direct: { ...demoDirect('connected'), peer: false, chosen: false, attempt: null, figures: { direct: null, relay: null } },
    }
    devices.value.push(device)
    tryNow(device.id)
    return { ok: true as const, device }
  }

  return { devices, shown, scenarios, setScenario, measure, tryNow, setRoute, rename, revoke, claim }
}
