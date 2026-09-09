import type { Conversation } from '../state/types'
import { useProduct } from '../state/product'

/** Resolve display metadata from the canonical target and account snapshot. */
export function executionFor(conversation: Conversation) {
  const snapshot = useProduct().snapshot
  const target = conversation.target
  const workspace =
    target.kind === 'workspace'
      ? snapshot?.workspaces.find((workspace) => workspace.id === target.workspaceId)
      : null
  const deviceId =
    target.kind === 'device'
      ? target.deviceId
      : (workspace?.deviceId ?? snapshot?.cloud?.device?.id ?? null)
  const device = snapshot?.devices.find((device) => device.id === deviceId)
  const kind =
    target.kind === 'cloud' || device?.kind === 'managed'
      ? ('cloud' as const)
      : ('device' as const)
  return {
    deviceId,
    kind,
    name: kind === 'cloud' ? 'Cloud' : (device?.name ?? 'Unavailable device'),
    path: target.kind === 'device' ? target.path : (workspace?.path ?? null),
    workspaceName: workspace?.name ?? null,
    online:
      kind === 'cloud'
        ? snapshot?.cloud?.state === 'running'
        : device?.online === true,
  }
}
