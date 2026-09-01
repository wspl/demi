import type { AgentServer } from '@demicodes/agent'
import type { ControlService, PrevTarget, WorkspaceRecord } from '../storage/control'

export type SwitchTargetResult =
  | { outcome: 'switched' }
  | { outcome: 'noop' }
  | { outcome: 'conversation_not_found' }
  | { outcome: 'workspace_not_found' }
  | { outcome: 'turn_in_flight' }
  | { outcome: 'conflict' }

export interface SwitchTargetDeps {
  control: ControlService
  agentServer: AgentServer
}

/**
 * The one generic switch mechanism (demi-next.md § Session and host model):
 * refused mid-turn, compare-and-set so concurrent switches have one winner,
 * departed target recorded in the prev slot (unannounced), files untouched.
 */
export async function switchConversationTarget(
  deps: SwitchTargetDeps,
  conversationId: string,
  toWorkspaceId: string | null,
): Promise<SwitchTargetResult> {
  const conversation = await deps.control.getConversation(conversationId)
  if (!conversation) return { outcome: 'conversation_not_found' }
  if (conversation.workspaceId === toWorkspaceId) return { outcome: 'noop' }

  let toWorkspace: WorkspaceRecord | null = null
  if (toWorkspaceId !== null) {
    toWorkspace = await deps.control.getWorkspace(toWorkspaceId)
    if (!toWorkspace || toWorkspace.userId !== conversation.userId) return { outcome: 'workspace_not_found' }
  }

  const phase = deps.agentServer.sessionPhase(conversationId)
  if (phase !== null && phase !== 'idle') return { outcome: 'turn_in_flight' }

  const prevTarget = await departedTarget(deps.control, conversation.workspaceId)
  const won = await deps.control.switchConversationWorkspace(conversationId, conversation.workspaceId, toWorkspaceId, {
    target: prevTarget,
    announced: false,
  })
  return won ? { outcome: 'switched' } : { outcome: 'conflict' }
}

async function departedTarget(control: ControlService, workspaceId: string | null): Promise<PrevTarget> {
  if (workspaceId === null) return { kind: 'virtual' }
  const workspace = await control.getWorkspace(workspaceId)
  if (!workspace) return { kind: 'virtual' }
  return { kind: 'workspace', deviceId: workspace.deviceId, path: workspace.path }
}
