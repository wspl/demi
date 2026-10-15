import type { MoveLock } from '@demicodes/web-ui/hosts/move-question'
import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import type { Conversation, Device } from '../state/types'
import { useProduct } from '../state/product'
import { directPath } from '../direct'
import { cloudDeviceIn, deviceMarkOf, hostKindOf, hostLabel } from '../state/hosts'

/** Resolve display metadata from the canonical target and account snapshot. */
export function executionFor(conversation: Pick<Conversation, 'target' | 'cwd'>) {
  const snapshot = useProduct().snapshot
  const target = conversation.target
  const workspace =
    target.kind === 'workspace'
      ? snapshot?.workspaces.find((workspace) => workspace.id === target.workspaceId)
      : null
  const deviceId =
    target.kind === 'device'
      ? target.deviceId
      : (workspace?.deviceId
        ?? cloudDeviceIn(snapshot?.devices)?.id
        ?? null)
  const device = snapshot?.devices.find((device) => device.id === deviceId)
  const kind = target.kind === 'cloud' ? ('cloud' as const) : hostKindOf(device)
  return {
    deviceId,
    kind,
    name: hostLabel(kind, device),
    /** Where the work runs: the backend's resolved directory, for every kind of target. */
    path: conversation.cwd,
    /** The directory the reader chose; null while the Cloud runs in its own session directory. */
    directory: target.kind === 'device' ? target.path : (workspace?.path ?? null),
    workspaceName: workspace?.name ?? null,
    /** Whether the Host's runner serves it: a paired device's, or a running Cloud's. */
    state: device?.state ?? ('offline' as const),
    /** The device's mark; null for the Cloud and a device the account no longer lists. */
    mark: deviceMarkOf(device, snapshot?.devices ?? []),
  }
}

/**
 * Why the conversation's move controls are unavailable, if they are
 * (`product.md` § Where a conversation runs): it is archived, or a change of
 * it is under way. Work that runs does not lock them; moving asks to stop it
 * first.
 */
export function moveLock(
  conversation: Pick<Conversation, 'id' | 'archived'>,
  pendingChanges: readonly string[],
): MoveLock | undefined {
  if (conversation.archived) {
    return 'archived'
  }
  return pendingChanges.includes(conversation.id) ? 'moving' : undefined
}

/** The Host a conversation runs on, as Run On checks it. */
export function primaryHostOf(execution: ReturnType<typeof executionFor>): HostMenuHost {
  return {
    id: execution.deviceId ?? 'cloud',
    name: execution.name,
    kind: execution.kind,
    state: execution.state,
    mark: execution.mark,
  }
}

/** The user's paired devices as Run On lists them, each with the path this page last reached it by; none for one it has not used this session. */
export function hostDeviceOptions(devices: readonly Device[]): HostDeviceOption[] {
  return devices.map((device) => ({
    id: device.id,
    name: device.name,
    state: device.state,
    mark: device.mark,
    path: directPath(device.id),
  }))
}
