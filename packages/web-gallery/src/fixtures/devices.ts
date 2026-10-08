import { onBeforeUnmount, ref } from 'vue'
import type { DeviceRoute, DirectAttempt, DirectStatus, PathFigures } from '@demicodes/web-ui/devices/direct'
import type { SettingsDevice } from '@demicodes/web-ui/settings/types'
import { demoDeviceReport, demoDeviceStart } from './device-installation'
import { ago } from './time'

/**
 * The paths a demo device's attempts take, one per state its page shows
 * (`direct-channel.md` § What the user sees), and the devices of the
 * Devices specimens, whose controls act on them as the product's do: Try
 * Again runs an attempt that ends as the device's path says, the route
 * changes how it is reached, and Rename… and Revoke… change the list.
 */
export type DirectScenario =
  | 'connected'
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
  { value: 'connected', label: 'Direct' },
  { value: 'slower', label: 'Slower' },
  { value: 'serverOnly', label: 'Server Only' },
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
/** A direct path on one network. */
const LOCAL: PathFigures = { latencyMs: 1.8, loss: 0 }
/** A direct path over a congested link, slower than the server's. */
const CONGESTED: PathFigures = { latencyMs: 620, loss: 0.06 }

/** Whether an attempt of `scenario` connects. */
const connects = (scenario: DirectScenario) => scenario === 'connected' || scenario === 'slower' || scenario === 'dropped'

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
    pair: connected ? { browser: null, device: '192.168.1.20:61204' } : null,
    permission: scenario === 'blocked' ? 'denied' : 'granted',
  }
}

/** How a device of `scenario` is reached, its last attempt a few minutes ago. */
export function demoDirect(scenario: DirectScenario): DirectStatus {
  const now = Date.now()
  const peer = scenario === 'connected' || scenario === 'slower'
  return {
    route: scenario === 'serverOnly' ? 'server' : 'automatic',
    crossing: scenario !== 'notOffered',
    permission: scenario === 'blocked' ? 'denied' : 'granted',
    peer,
    chosen: scenario === 'connected',
    trying: false,
    attempt: scenario === 'serverOnly' ? null : demoAttempt(scenario, now - minutes(3)),
    nextAt: peer || scenario === 'serverOnly' || scenario === 'blocked' ? null : new Date(now + minutes(7)).toISOString(),
    figures: {
      direct: scenario === 'connected' ? LOCAL : scenario === 'slower' ? CONGESTED : null,
      relay: RELAY,
    },
  }
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
      device.direct = { ...status, route: device.direct.route, attempt: demoAttempt(ends, startedAt) }
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
      // Prefer Direct uses a standing peer whatever Automatic found.
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
      direct: { ...demoDirect('connected'), peer: false, chosen: false, attempt: null, figures: { direct: null, relay: RELAY } },
    }
    devices.value.push(device)
    tryNow(device.id)
    return { ok: true as const, device }
  }

  return { devices, shown, scenarios, setScenario, tryNow, setRoute, rename, revoke, claim }
}
