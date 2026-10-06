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
import { DeviceDirect, type DirectState, type Permission } from './device'
import { connectPeer } from './peer'
import { DeviceSignaling } from './signaling'

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
  if (execution.kind !== 'device' || !execution.deviceId || !execution.online || !execution.path)
    return null
  return {
    device: use(execution.deviceId),
    scope: { conversation: conversationId, cwd: execution.path },
  }
}

/**
 * What Settings → Devices says of the path to device `deviceId`
 * (`direct-channel.md` § What the user sees): `connected` while this page
 * has a direct channel to it, `blocked` while the browser reports that it
 * blocks local network access, and nothing otherwise.
 */
export function directNote(deviceId: string): 'connected' | 'blocked' | undefined {
  follow()
  if (permission.value === 'denied')
    return 'blocked'
  return states.get(deviceId)?.choice === 'direct' ? 'connected' : undefined
}

/** The device's choice and signaling, started on its first use while its runner is connected. */
function use(deviceId: string): DeviceDirect {
  const known = used.get(deviceId)
  if (known)
    return known.direct
  follow()
  const state = states.get(deviceId) ?? reactive({ choice: 'relay' as const, blocked: false })
  states.set(deviceId, state)
  // The first open of the socket starts the first attempt, and each open
  // after one closed is a reason to try again.
  const signaling = new DeviceSignaling(deviceId, () => direct.tryNow())
  const direct = new DeviceDirect(
    {
      ready: () => signaling.open,
      connect: (signal) => connectPeer((sdp) => signaling.offer(sdp), signal),
      after: (ms, run) => {
        const timer = setTimeout(run, ms)
        return () => clearTimeout(timer)
      },
    },
    state,
  )
  direct.setPermission(permission.value)
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
      () => (product.snapshot?.devices ?? []).filter((device) => device.online).map((device) => device.id),
      (online) => {
        for (const deviceId of [...used.keys()]) {
          if (!online.includes(deviceId))
            release(deviceId)
        }
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
