import { markRaw } from 'vue'
import type { WorkflowChoice } from '@demicodes/protocol'
import { NO_COST } from '@demicodes/web-ui/agent/context-cost'
import { initialModelSettings, type ModelSettings } from '@demicodes/web-ui/agent/model-selection'
import { Slot } from '@demicodes/web-ui/infra/slot'
import { EMPTY_TRANSCRIPT } from '@demicodes/web-ui/transport/protocol'
import type { AttachedHost, ConversationMetadata, ConversationStatus, ConversationSummary } from '../api/generated/web-api'
import type { Conversation } from '../state/types'
import { newRecord } from './record'

/**
 * What a conversation's record says of it before the page holds anything of
 * its session: the sidebar's status, the entry's fields, and the whole
 * conversation as it starts, which the store holds for each conversation and a
 * view link's page for its one.
 */

/**
 * The conversation shows its metadata as the page holds it now, with the
 * page's changes over it: the model settings its record holds, whichever
 * page or device changed them last, its directory, workflow and plugins.
 * A record without a model leaves the page's own settings, which a send
 * writes to it. Its hosts come whole with it, under their revision, and
 * one no newer than the page's changes nothing.
 */
export function followMetadata(conversation: Conversation): void {
  const shown = conversation.metadata.shown.value
  if (!shown) {
    return
  }
  conversation.cwd = shown.cwd
  conversation.workflow = shown.workflow
  conversation.pluginsChanged = shown.pluginsChanged
  conversation.turnSpeed = shown.turnSpeed
  conversation.suggestedReplies = shown.suggestedReplies
  if (shown.model) {
    conversation.model = { ...shown.model }
  }
  conversation.attachedHosts.replace(shown.hosts.hosts, { revisions: { '': shown.hosts.revision } })
}

/** The sidebar's status of a conversation whose record says `status`. */
export function summaryStatus(status: ConversationStatus): Conversation['status'] {
  // A turn Demi resumes after a restart runs again without the user.
  if (status === 'running' || status === 'compacting' || status === 'resuming') {
    return 'active'
  }
  // A turn the process died under is a failure, like a provider's; only the
  // user's own Stop is an abort.
  if (status === 'error' || status === 'interrupted') {
    return 'error'
  }
  if (status === 'stopped') {
    return 'aborted'
  }
  return status === 'completed' ? 'done' : 'idle'
}

/** What the sidebar's entry `record` sets on its conversation. */
export function listEntry(
  record: Pick<
    ConversationSummary,
    | 'id'
    | 'workspaceId'
    | 'title'
    | 'pinned'
    | 'archived'
    | 'archivedAt'
    | 'scheduled'
    | 'target'
    | 'worktree'
    | 'revision'
    | 'readRevision'
    | 'unread'
    | 'unreadPermissionRequests'
    | 'createdAt'
    | 'updatedAt'
  >,
) {
  return {
    id: record.id,
    workspaceId: record.workspaceId,
    title: record.title,
    pinned: record.pinned,
    archived: record.archived,
    archivedAt: record.archivedAt,
    scheduled: record.scheduled,
    target: record.target,
    projectId:
      record.target.kind === 'project' ? record.target.projectId : null,
    worktree: record.worktree ?? undefined,
    revision: record.revision,
    readRevision: record.readRevision,
    unread: record.unread,
    // The sidebar's mark settles once the request is read, though it still waits.
    needsYou: record.unreadPermissionRequests > 0,
    createdAt: record.createdAt,
    updatedAt: record.updatedAt,
  }
}

/**
 * A conversation as the sidebar's entry `record` lists it, with what only
 * the page knows of one without a record: `local`'s model settings,
 * workflow and directory.
 */
export function newConversation(
  record: Omit<ConversationSummary, 'panel'>,
  local: { model?: ModelSettings, workflow?: WorkflowChoice, cwd?: string } = {},
): Conversation {
  return {
    ...listEntry(record),
    persistence: 'synced',
    status: summaryStatus(record.status),
    metadata: markRaw(new Slot<ConversationMetadata>()),
    metadataRun: null,
    agentsHeld: null,
    cwd: local.cwd ?? '',
    pluginsChanged: false,
    turnSpeed: null,
    suggestedReplies: null,
    workflows: null,
    history: EMPTY_TRANSCRIPT,
    shownAt: null,
    instructions: [],
    summaries: {},
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    starting: null,
    pendingCalls: [],
    waitingSince: null,
    requestOutputs: {},
    model: local.model ?? initialModelSettings(),
    workflow: local.workflow ?? { type: 'default' },
    lastError: null,
    draft: '',
    draftShown: 0,
    comments: [],
    record: newRecord(),
    savedWrites: [],
    files: [],
    attachmentIds: [],
    pendingSends: [],
    delivered: [],
    removing: [],
    messageEdit: null,
    scroll: null,
    attachedHosts: markRaw(new Slot<AttachedHost[]>([])),
    attachmentDescriptions: {},
    replyBlockFacts: {},
    subagents: [],
    shownSubagent: null,
    turnStartedAt: null,
    sides: [],
    terminals: [],
    load: 'loading',
    pendingAction: null,
    resuming: false,
    failures: {},
    contextUsage: null,
    cost: NO_COST,
    charts: {},
    recent: { hour: null, day: null },
  }
}
