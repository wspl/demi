import type { ControlService, ConversationRecord, ExecutionTarget } from '../storage/control'

/**
 * The resolution order of `sessions-and-targets.md` § The three states:
 * workspace if set, else the session-bound managed host, else hostless. The
 * one place the conversation row is read as a target.
 */
export async function resolveExecutionTarget(control: ControlService, conversation: ConversationRecord): Promise<ExecutionTarget> {
  if (conversation.workspaceId !== null) {
    const workspace = await control.getWorkspace(conversation.workspaceId)
    if (workspace) return { kind: 'workspace', workspaceId: workspace.id, deviceId: workspace.deviceId, path: workspace.path }
  }
  if (conversation.hostDeviceId !== null) return { kind: 'host', deviceId: conversation.hostDeviceId }
  return { kind: 'hostless' }
}

/** The device a target runs on, if it runs on one. */
export function targetDeviceId(target: ExecutionTarget): string | null {
  return target.kind === 'hostless' ? null : target.deviceId
}
