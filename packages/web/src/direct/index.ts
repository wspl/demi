/**
 * The page's direct channels (`crates/runner-direct/src/lib.rs`), one per
 * paired device it uses: the device's signaling socket and the choice of path
 * to it, while its runner is connected. A conversation's operations go to the
 * device its summary names as its Host, in the directory the summary names, so
 * they stop going to a device once the conversation moved.
 *
 * The page keeps one choice per device, `relay` or `direct`, from the device's
 * route:
 *
 * - Automatic makes a peer and chooses `direct` while it is connected and its
 *   last measurement found it no worse than the relay. A connected peer is not
 *   always the better path: across networks the pair may run over a congested
 *   or lossy link.
 * - Prefer P2P is for a user who knows the direct path suits them.
 * - Relay Only is for one whose network or policy makes the attempts unwelcome;
 *   it makes no peer.
 *
 * Each operation chooses as it starts and finishes there, while an open stream
 * or watch reopens on the other path when the choice changes.
 *
 * - A channel that closed before it opened while its peer stays connected says
 *   nothing about the path and is tried once more.
 * - Any other failure sets the choice to `relay` at once and the operation runs
 *   again there, a write once, since one that did not finish leaves the file as
 *   it was.
 *
 * While the choice is `relay`, the page makes a new peer:
 *
 * - when the browser's local network permission changes;
 * - when it comes back online;
 * - when the device's runner connects again;
 * - when its signaling socket reconnects;
 * - otherwise after 1, 2, 5 and then every 10 minutes, one attempt at a time.
 *
 * The runners keep the same schedule in Rust
 * (`crates/runner-protocol/src/forward.rs`): the two cannot share code, so both
 * follow its numbers. The relay never closes because the direct channel opened.
 *
 * A page measures the two paths only when something reads the result (a peer
 * connecting, the device's page becoming visible, the user asking), so an idle
 * page sends nothing. Latency and loss decide the route; throughput is not
 * measured, since a speed test would move megabytes through the user's server
 * to show a figure nothing acts on.
 */
import { effectScope, reactive, ref, watch } from 'vue'
import { z } from 'zod'
import { defaultWindow } from '@vueuse/core'
import { browserOnline } from '@demicodes/web-ui/transport/liveness'
import { useConversations } from '../conversation/store'
import { useProduct } from '../state/product'
import { executionFor } from '../targets/execution'
import { DEVICE_PATHS, statusPath, type DevicePath, type PathStatus, type DeviceRoute, type DirectStatus } from '@demicodes/web-ui/devices/direct'
import { DeviceDirect, directState, type DirectState, type Permission } from './device'
import { DeviceLive } from './live'
import { DeviceMeter, meterState, type MeterState } from './meter'
import { connectPeer } from './peer'
import { DeviceSignaling, isBusy } from './signaling'

/** A device in use, with its signaling socket, its peer for live views and the measuring of its paths. */
interface UsedDevice {
  direct: DeviceDirect
  live: DeviceLive
  signaling: DeviceSignaling
  meter: DeviceMeter
}

/** The devices in use, by id. */
const used = new Map<string, UsedDevice>()
/** What the views show of each device's path, by id, kept while the page lives. */
const states = reactive(new Map<string, DirectState>())
/** What the views show of each device's measured paths, by id. */
const meters = reactive(new Map<string, MeterState>())
/** The browser's local network permission, where it reports one. */
const permission = ref<Permission | null>(null)

/** Where a conversation's operations go directly: its device's path, and the conversation and directory each header names. */
export interface DirectRoute {
  device: DeviceDirect
  /** The device's peer for live views, used while the device's choice is `direct`. */
  live: DeviceLive
  scope: { conversation: string; cwd: string }
}

/**
 * The direct route of `conversationId`, while its summary names a paired
 * device whose runner is connected; using it starts the device's
 * signaling. None for a conversation on the Cloud, or one not known yet.
 */
export function directRoute(conversationId: string): DirectRoute | null {
  const summary = useConversations().items.find((item) => item.id === conversationId)
  if (!summary)
    return null
  const execution = executionFor(summary)
  if (execution.kind !== 'device' || !execution.deviceId || execution.state !== 'online' || !execution.path)
    return null
  const device = use(execution.deviceId)
  return {
    device: device.direct,
    live: device.live,
    scope: { conversation: conversationId, cwd: execution.path },
  }
}

/**
 * Starts the use of paired device `deviceId` while its runner is
 * connected, so that Settings → Devices says how this page reaches it even
 * before a conversation used it.
 */
export function watchDirect(deviceId: string): void {
  const device = useProduct().snapshot?.devices.find((candidate) => candidate.id === deviceId)
  if (device?.kind === 'user' && device.state === 'online')
    use(deviceId)
}

/** What Settings → Devices shows of the path to paired device `deviceId` (`packages/web-ui/src/devices/direct.ts`). */
export function directStatus(deviceId: string): DirectStatus {
  const product = useProduct()
  const device = product.snapshot?.devices.find((candidate) => candidate.id === deviceId)
  const state = states.get(deviceId) ?? directState()
  const measured = meters.get(deviceId) ?? meterState()
  return {
    route: device?.route ?? 'automatic',
    crossing: (product.snapshot?.stunUrls.length ?? 0) > 0,
    permission: permission.value,
    ...pathStatus(state),
    trying: state.trying,
    nextAt: state.nextAt === null ? null : new Date(state.nextAt).toISOString(),
    figures: measured.figures,
    measuring: measured.measuring,
  }
}

/**
 * The path this page uses for paired device `deviceId`, as the host menu
 * names it (`DEVICE_PATH_LABEL`): "LAN". The last path found stays while
 * the page lives; none for a device this page never used, whose menu row
 * shows the one this browser found last time (`lastDirectPath`).
 */
export function directPath(deviceId: string): DevicePath | null {
  const state = states.get(deviceId)
  return state ? statusPath(pathStatus(state)) : null
}

/** What of a device's state decides the path the page uses. */
function pathStatus(state: DirectState): PathStatus {
  return { peer: state.peer, chosen: state.choice === 'direct', attempt: state.attempt }
}

/** Where this browser keeps the path it found to paired device `deviceId`. */
const lastPathKey = (deviceId: string) => `demi.devicePath.${deviceId}`
const lastPathSchema = z.enum(DEVICE_PATHS)

/**
 * The path this browser found to paired device `deviceId` last time, in
 * this page or an earlier one, which the host menu shows fainter until the
 * page finds a current one; none for a device it never reached.
 */
export function lastDirectPath(deviceId: string): DevicePath | null {
  try {
    const kept = localStorage.getItem(lastPathKey(deviceId))
    return kept === null ? null : lastPathSchema.parse(kept)
  } catch (error) {
    // The browser's storage is optional, and an entry that is not a path
    // is shown as none rather than repaired.
    console.warn('Could not read the last path to a device', error)
    return null
  }
}

/** Keeps `path` as the one this browser found to device `deviceId` last. */
function keepLastPath(deviceId: string, path: DevicePath): void {
  try {
    localStorage.setItem(lastPathKey(deviceId), path)
  } catch (error) {
    // Private browsing and storage quotas only cost the faint path next time.
    console.warn('Could not keep the last path to a device', error)
  }
}

/**
 * Measures paired device `deviceId`'s paths once, starting its use while
 * its runner is connected, as its page shows or Measure asks
 * (`packages/web/src/direct/measure.ts`). A measurement that runs is
 * joined.
 */
export function measureDirect(deviceId: string): void {
  watchDirect(deviceId)
  void used.get(deviceId)?.meter.measure()
}

/** Makes a new attempt to device `deviceId` at once, as Try Again asks. */
export function tryDirect(deviceId: string): void {
  used.get(deviceId)?.direct.tryNow()
}

/** The device's choice and signaling, started on its first use while its runner is connected. */
function use(deviceId: string): UsedDevice {
  const known = used.get(deviceId)
  if (known)
    return known
  const product = useProduct()
  const state = states.get(deviceId) ?? reactive(directState())
  states.set(deviceId, state)
  // The first open of the socket starts the first attempt, and each open
  // after one closed is a reason to try again; a peer the backend closed is
  // made again at once; a socket that dropped took the peers with it, so the
  // page uses the relay until a new channel is made. A live peer the
  // backend closed is made again by the next live view.
  const signaling = new DeviceSignaling(
    deviceId,
    () => direct.tryNow(),
    (peer) => (peer === 'files' ? direct.reintroduce() : live.lost()),
    () => {
      live.lost()
      direct.failed()
    },
  )
  const after = (ms: number, run: () => void) => {
    const timer = setTimeout(run, ms)
    return () => clearTimeout(timer)
  }
  const direct = new DeviceDirect(
    {
      ready: () => signaling.open,
      connect: (signal) =>
        connectPeer({
          signaling: signaling.peer('files'),
          stunUrls: product.snapshot?.stunUrls ?? [],
          permission: () => permission.value,
          busy: isBusy,
          signal,
        }),
      after,
      // Automatic decides from a measurement of the new peer's path.
      connected: () => void meter.measure(),
    },
    state,
  )
  const live = new DeviceLive({
    connect: (signal) =>
      connectPeer({
        signaling: signaling.peer('live'),
        stunUrls: product.snapshot?.stunUrls ?? [],
        permission: () => permission.value,
        busy: isBusy,
        live: true,
        signal,
      }),
    closed: () => signaling.close('live'),
    after,
  })
  // The live peer serves only while the choice is `direct`.
  direct.onChange((choice) => {
    if (choice !== 'direct')
      live.close()
  })
  const route = (): DeviceRoute => product.snapshot?.devices.find((device) => device.id === deviceId)?.route ?? 'automatic'
  const measured = meters.get(deviceId) ?? reactive(meterState())
  meters.set(deviceId, measured)
  const meter = new DeviceMeter(direct, signaling, route, measured)
  direct.setPermission(permission.value)
  direct.setRoute(route())
  const device = { direct, live, signaling, meter }
  used.set(deviceId, device)
  return device
}

/** Ends the use of a device whose runner went or that was revoked: its peer closes with its socket. */
function release(deviceId: string): void {
  const device = used.get(deviceId)
  if (!device)
    return
  used.delete(deviceId)
  device.meter.close()
  device.live.close()
  device.direct.stop()
  device.signaling.stop()
}

let following = false

/**
 * Follows, for the page's lifetime, from its start, what makes the page use
 * a device, try again or let it go: the conversation it shows, whose
 * device's peer is made as it shows, before any operation needs it
 * (`crates/runner-direct/src/lib.rs`); a runner leaving or
 * connecting again; the page coming back online; and the browser's local
 * network permission.
 */
export function followDirect(): void {
  if (following)
    return
  following = true
  const product = useProduct()
  effectScope(true).run(() => {
    // A runner that left ends the device's use; its next use, once the
    // runner connected again, starts anew and tries at once.
    watch(
      () => (product.snapshot?.devices ?? []).filter((device) => device.state === 'online').map((device) => device.id),
      (online) => {
        for (const deviceId of [...used.keys()]) {
          if (!online.includes(deviceId))
            release(deviceId)
        }
      },
    )
    // The device's route, which every page of the user's follows.
    watch(
      () => (product.snapshot?.devices ?? []).map((device) => [device.id, device.route] as const),
      (devices) => {
        for (const [deviceId, route] of devices)
          used.get(deviceId)?.direct.setRoute(route)
      },
    )
    // The conversation the page shows makes its device's peer before its
    // first operation needs it.
    const conversations = useConversations()
    watch(
      () => {
        const summary = conversations.items.find((item) => item.id === product.activeConversationId)
        const execution = summary ? executionFor(summary) : null
        return execution?.kind === 'device' && execution.state === 'online' ? execution.deviceId : null
      },
      (deviceId) => {
        if (deviceId)
          watchDirect(deviceId)
      },
      { immediate: true },
    )
    // Each path the page finds to a device is kept as the one found last;
    // one taken while an attempt is still being made is not found yet.
    watch(
      () => [...states].filter(([, state]) => !state.trying).map(([deviceId]) => [deviceId, directPath(deviceId)] as const),
      (found) => {
        for (const [deviceId, path] of found) {
          if (path && path !== lastDirectPath(deviceId))
            keepLastPath(deviceId, path)
        }
      },
      { immediate: true },
    )
    // Back online after an offline that held, as the page counts it.
    watch(browserOnline, (online) => {
      if (!online)
        return
      for (const device of used.values())
        device.direct.tryNow()
    })
  })
  void followPermission()
}

/**
 * Follows the browser's local network permission where the Permissions API
 * reports it; a change is a reason to try again, and a blocked one keeps
 * every device on the relay.
 */
async function followPermission(): Promise<void> {
  const status = await localNetworkPermission()
  if (!status)
    return
  const take = () => {
    permission.value = status.state
    for (const device of used.values())
      device.direct.setPermission(status.state)
  }
  take()
  status.addEventListener('change', take)
}

/** The local network permission's status, under the name the browser knows it by; none where it reports none. */
async function localNetworkPermission(): Promise<PermissionStatus | null> {
  const permissions = defaultWindow?.navigator.permissions
  if (!permissions)
    return null
  for (const name of ['local-network-access', 'local-network']) {
    try {
      // The names are not in the DOM's list of permission names yet; the
      // browsers that know them answer, the others refuse.
      return await permissions.query({ name } as unknown as PermissionDescriptor)
    } catch {
      // This browser does not know the name.
    }
  }
  return null
}
