import type { ConversationTitles } from './title'
import type { AgentServer } from '@demicodes/agent'
import type { RunnerRegistry } from '../runner/registry'
import type { ControlService, ConversationRecord } from '../storage/control'
import type { ConversationStores } from '../storage/conversation-store'
import { resolveExecutionTarget } from './execution-target'

export interface ConversationSummaryDeps {
  stores: ConversationStores
  server: AgentServer
  control: ControlService
  registry: Pick<RunnerRegistry, 'deviceIdentity'>
  titles: Pick<ConversationTitles, 'generating'>
}

/**
 * A conversation as the browser lists it: its record, its live status, and
 * `cwd`, the directory its work runs in, resolved the same way for a
 * device directory, a workspace and the Cloud, so the browser never derives
 * it. Live activity wins; a cold unfinished checkpoint is interrupted,
 * never reported as still running.
 */
export async function conversationSummary(
  conversation: ConversationRecord,
  deps: ConversationSummaryDeps
) {
  const { stores, server, control, registry } = deps
  const cwd = (await resolveExecutionTarget(control, registry, conversation)).path
  const summary = stores.summary(conversation.id)
  const live = server.sessionPhase(conversation.id)
  let status:
    'running' | 'compacting' | 'interrupted' | 'error' |
    'stopped' | 'completed' | 'idle' = 'idle'
  if (server.treeActive(conversation.id))
    status = live === 'compacting' ? 'compacting' : 'running'
  else if (!live && summary.phase !== 'idle')
    status = 'interrupted'
  else if (summary.last === 'error')
    status = 'error'
  else if (summary.last === 'abort')
    status = 'stopped'
  else if (summary.last === 'response')
    status = 'completed'
  return {
    ...conversation,
    cwd,
    status,
    revision: summary.revision,
    unread: summary.revision > conversation.readRevision,
    // Whether asking for a new title could say anything the last one did not (`product.md` § Conversation titles).
    titleCurrent: conversation.userMessages <= conversation.titledMessages,
    titleGenerating: deps.titles.generating(conversation.id),
  }
}
