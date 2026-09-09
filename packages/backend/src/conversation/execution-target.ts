import type { RunnerRegistry } from '../runner/registry'
import type {
  ControlService,
  ConversationRecord,
  ExecutionTarget
} from '../storage/control'

export const CLOUD_HOME = '/home/demi'
export function cloudSessionDirectory(
  conversationId: string,
  home = CLOUD_HOME
): string {
  return `${home}/sessions/${conversationId}`
}

/**
 * Resolving metadata never starts a machine. Missing internal references are
 * errors.
 */
export async function resolveExecutionTarget(
  control: ControlService,
  registry: Pick<RunnerRegistry, 'deviceIdentity'>,
  conversation: ConversationRecord
): Promise<ExecutionTarget> {
  const target = conversation.target
  if (target.kind === 'workspace') {
    const workspace = await control.getWorkspace(target.workspaceId)
    if (!workspace || workspace.userId !== conversation.userId)
      throw new Error('Invalid workspace target')
    return {
      kind: 'workspace',
      workspaceId: workspace.id,
      deviceId: workspace.deviceId,
      path: workspace.path
    }
  }
  if (target.kind === 'device')
    return target
  const deviceId = (await control.getManagedDevice(conversation.userId))?.id ??
    null
  return {
    kind: 'cloud',
    deviceId,
    path: cloudSessionDirectory(
      conversation.id,
      deviceId ? registry.deviceIdentity(deviceId)?.homeDir : undefined
    )
  }
}
