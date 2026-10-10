import type { HostDeviceOption, HostMenuHost } from '@demicodes/web-ui/hosts/types'
import type { Conversation, Device } from '../state/types'
import { useProduct } from '../state/product'
import { directPath } from '../direct'

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
        ?? snapshot?.devices.find((device) => device.kind === 'managed')?.id
        ?? null)
  const device = snapshot?.devices.find((device) => device.id === deviceId)
  const kind =
    target.kind === 'cloud' || device?.kind === 'managed'
      ? ('cloud' as const)
      : ('device' as const)
  return {
    deviceId,
    kind,
    name: kind === 'cloud' ? 'Cloud' : (device?.name ?? 'Unavailable device'),
    /** Where the work runs: the backend's resolved directory, for every kind of target. */
    path: conversation.cwd,
    /** The directory the reader chose; null while the Cloud runs in its own session directory. */
    directory: target.kind === 'device' ? target.path : (workspace?.path ?? null),
    workspaceName: workspace?.name ?? null,
    /** Whether the Host's runner serves it: a paired device's, or a running Cloud's. */
    state: device?.state ?? ('offline' as const),
  }
}

/**
 * Whether the conversation cannot move now (`product.md` § Where a
 * conversation runs): its work runs, it is archived, or a change of it is
 * under way.
 */
export function moveLocked(
  conversation: Pick<Conversation, 'id' | 'phase' | 'archived'>,
  pendingChanges: readonly string[],
): boolean {
  return conversation.phase !== 'idle' || conversation.archived || pendingChanges.includes(conversation.id)
}

/** The Host a conversation runs on, as Run On checks it. */
export function primaryHostOf(execution: ReturnType<typeof executionFor>): HostMenuHost {
  return {
    id: execution.deviceId ?? 'cloud',
    name: execution.name,
    kind: execution.kind,
    state: execution.state,
  }
}

/** The user's paired devices as Run On lists them, each with the path this page last reached it by; none for one it has not used this session. */
export function hostDeviceOptions(devices: readonly Device[]): HostDeviceOption[] {
  return devices.map((device) => ({
    id: device.id,
    name: device.name,
    state: device.state,
    path: directPath(device.id),
  }))
}
