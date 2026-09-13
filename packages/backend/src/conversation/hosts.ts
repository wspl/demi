import type { RunnerRegistry } from '../runner/registry'
import type { AttachedHostRecord, ControlService } from '../storage/control'
import { resolveExecutionTarget } from './execution-target'

/**
 * A host this conversation may dispatch to: its name, its device and the
 * directory commands start in.
 */
export interface ReachableHost {
  name: string
  deviceId: string
  path: string
  role: 'main' | 'attached'
}

/**
 * The hosts browser access and `shell --host` accept (`sessions-and-targets.md` § Attached
 * hosts): the main host and the attached ones. The main host's shell starts
 * in the conversation's directory there, an attached host's where the last
 * shell there ended, its home before one ran. The one place the check lives.
 */
export async function reachableHosts(
  deps: { control: ControlService; registry: Pick<RunnerRegistry, 'deviceIdentity'> },
  conversationId: string
): Promise<ReachableHost[]> {
  const conversation = await deps.control.getConversation(conversationId)
  if (!conversation)
    return []
  const target = await resolveExecutionTarget(
    deps.control,
    deps.registry,
    conversation
  )
  const hosts: ReachableHost[] = []
  const mainDeviceId = target.deviceId
  if (mainDeviceId !== null) {
    const device = await deps.control.getDevice(mainDeviceId)
    hosts.push({
      name: device?.name ?? mainDeviceId,
      deviceId: mainDeviceId,
      path: target.path,
      role: 'main'
    })
  }
  for (const attached of await deps.control.listAttachedHosts(conversationId)) {
    if (attached.deviceId === mainDeviceId)
      continue
    hosts.push({
      name: attached.name,
      deviceId: attached.deviceId,
      path: attachedDirectory(deps, attached),
      role: 'attached'
    })
  }
  return hosts
}

function attachedDirectory(
  deps: { control: ControlService; registry: Pick<RunnerRegistry, 'deviceIdentity'> },
  attached: AttachedHostRecord
): string {
  return attached.cwd ??
    deps.registry.deviceIdentity(attached.deviceId)?.homeDir ??
    ''
}

