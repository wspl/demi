import type { AgentServer } from '@demicodes/agent'
import type { ControlService, ExecutionTarget, WorkspaceRecord } from '../storage/control'
import { resolveExecutionTarget, targetDeviceId } from './execution-target'

export type SwitchTargetResult =
  | { outcome: 'switched' }
  | { outcome: 'noop' }
  | { outcome: 'conversation_not_found' }
  | { outcome: 'workspace_not_found' }
  | { outcome: 'no_hostless_entrance' }
  | { outcome: 'turn_in_flight' }
  | { outcome: 'conflict' }

export interface SwitchTargetDeps {
  control: ControlService
  agentServer: AgentServer
}

/**
 * The one generic switch mechanism (`sessions-and-targets.md` § Switching),
 * user-initiated from the target picker: refused mid-turn, compare-and-set so
 * concurrent switches have one winner, the departed device attached to the
 * conversation at the directory it was left at, the arriving device detached,
 * the switch recorded for the next turn's announcement. Files are never
 * moved. A session-bound managed host has no hostless entrance.
 */
export async function switchConversationTarget(
  deps: SwitchTargetDeps,
  conversationId: string,
  toWorkspaceId: string | null,
): Promise<SwitchTargetResult> {
  const conversation = await deps.control.getConversation(conversationId)
  if (!conversation) return { outcome: 'conversation_not_found' }
  if (conversation.workspaceId === toWorkspaceId && conversation.hostDeviceId === null) return { outcome: 'noop' }

  let toWorkspace: WorkspaceRecord | null = null
  if (toWorkspaceId !== null) {
    toWorkspace = await deps.control.getWorkspace(toWorkspaceId)
    if (!toWorkspace || toWorkspace.userId !== conversation.userId) return { outcome: 'workspace_not_found' }
  } else if (conversation.hostDeviceId !== null) {
    return { outcome: 'no_hostless_entrance' }
  }

  const phase = deps.agentServer.sessionPhase(conversationId)
  if (phase !== null && phase !== 'idle') return { outcome: 'turn_in_flight' }

  const from = await resolveExecutionTarget(deps.control, conversation)
  const to: ExecutionTarget = toWorkspace
    ? { kind: 'workspace', workspaceId: toWorkspace.id, deviceId: toWorkspace.deviceId, path: toWorkspace.path }
    : { kind: 'hostless' }
  const departedDeviceId = targetDeviceId(from)
  const won = await deps.control.switchConversationTarget(
    conversationId,
    { workspaceId: conversation.workspaceId, hostDeviceId: conversation.hostDeviceId },
    { workspaceId: toWorkspaceId, hostDeviceId: null },
    { from, to },
    {
      departed: departedDeviceId === null ? null : { deviceId: departedDeviceId, cwd: from.kind === 'workspace' ? from.path : null },
      arrivingDeviceId: targetDeviceId(to),
    },
  )
  return won ? { outcome: 'switched' } : { outcome: 'conflict' }
}
