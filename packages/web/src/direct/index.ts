/**
 * The page's direct channels (`direct-channel.md`), one per paired device
 * it uses: the device's signaling socket and the choice of path to it,
 * while its runner is connected. A conversation's operations go to the
 * device its summary names as its Host, in the directory the summary
 * names, so they stop going to a device once the conversation moved.
 */
import { effectScope, reactive, ref, watch } from 'vue'
import { defaultWindow, useEventListener } from '@vueuse/core'
import { useConversations } from '../conversation/store'
import { useProduct } from '../state/product'
import { executionFor } from '../targets/execution'
import type { DirectStatus } from '@demicodes/web-ui/devices/direct'
import { DeviceDirect, directState, type DirectState, type Permission } from './device'
import { connectPeer } from './peer'
import { DeviceSignaling, isBusy } from './signaling'

/** A device in use, with its signaling socket. */
interface UsedDevice {
  direct: DeviceDirect
  signaling: DeviceSignaling
}

/** The devices in use, by id. */
const used = new Map<string, UsedDevice>()
/** What the views show of each device's path, by id, kept while the page lives. */
const states = reactive(new Map<string, DirectState>())
/** The browser's local network permission, where it reports one. */
const permission = ref<Permission | null>(null)

/** Where a conversation's operations go directly: its device's path, and the conversation and directory each header names. */
export interface DirectRoute {
  device: DeviceDirect
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
  return {
    device: use(execution.deviceId),
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

/** What Settings → Devices shows of the path to paired device `deviceId` (`direct-channel.md` § What the user sees). */
export function directStatus(deviceId: string): DirectStatus {
  const product = useProduct()
  const device = product.snapshot?.devices.find((candidate) => candidate.id === deviceId)
  follow()
  const state = states.get(deviceId) ?? directState()
  return {
    enabled: device?.direct ?? true,
    crossing: (product.snapshot?.stunUrls.length ?? 0) > 0,
    permission: permission.value,
    connected: state.choice === 'direct',
    trying: state.trying,
    roundTripMs: null,
    attempt: state.attempt,
    nextAt: state.nextAt === null ? null : new Date(state.nextAt).toISOString(),
  }
}

/** Makes a new attempt to device `deviceId` at once, as Try Now asks. */
export function tryDirect(deviceId: string): void {
  used.get(deviceId)?.direct.tryNow()
}

/** The round trip of the direct channel to device `deviceId`, in milliseconds; null while there is none. */
export async function directRoundTrip(deviceId: string): Promise<number | null> {
  const peer = used.get(deviceId)?.direct.current()
  return peer ? peer.roundTrip() : null
}

/** The device's choice and signaling, started on its first use while its runner is connected. */
function use(deviceId: string): DeviceDirect {
  const known = used.get(deviceId)
  if (known)
    return known.direct
  follow()
  const product = useProduct()
  const state = states.get(deviceId) ?? reactive(directState())
  states.set(deviceId, state)
  // The first open of the socket starts the first attempt, and each open
  // after one closed is a reason to try again; a peer the backend closed is
  // made again at once.
  const signaling = new DeviceSignaling(deviceId, () => direct.tryNow(), () => direct.reintroduce())
  const direct = new DeviceDirect(
    {
      ready: () => signaling.open,
      connect: (signal) =>
        connectPeer({
          signaling,
          stunUrls: product.snapshot?.stunUrls ?? [],
          permission: () => permission.value,
          busy: isBusy,
          signal,
        }),
      after: (ms, run) => {
        const timer = setTimeout(run, ms)
        return () => clearTimeout(timer)
      },
    },
    state,
  )
  direct.setPermission(permission.value)
  direct.setEnabled(product.snapshot?.devices.find((device) => device.id === deviceId)?.direct ?? true)
  used.set(deviceId, { direct, signaling })
  return direct
}

/** Ends the use of a device whose runner went or that was revoked: its peer closes with its socket. */
function release(deviceId: string): void {
  const device = used.get(deviceId)
  if (!device)
    return
  used.delete(deviceId)
  device.direct.stop()
  device.signaling.stop()
}

let following = false

/**
 * Follows, for the page's lifetime, what makes the page try again or let a
 * device go: its runner leaving or connecting again, the page coming back
 * online, and the browser's local network permission.
 */
function follow(): void {
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
    // The device's switch, which every page of the user's follows.
    watch(
      () => (product.snapshot?.devices ?? []).map((device) => [device.id, device.direct] as const),
      (devices) => {
        for (const [deviceId, enabled] of devices)
          used.get(deviceId)?.direct.setEnabled(enabled)
      },
    )
    useEventListener(defaultWindow, 'online', () => {
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
