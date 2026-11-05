import { computed, reactive, shallowRef, watch } from 'vue'
import type { JobModel } from '@demicodes/protocol'
import { ConversationRuntime } from '@demicodes/web-ui/agent/conversation-runtime'
import { latestBlocks } from '@demicodes/web-ui/agent/history'
import type { AgentRun, SubagentPhase } from '@demicodes/web-ui/agent/subagents'
import { configurePageConnection } from '@demicodes/web-ui/transport/page-connection'
import { createConversationClient } from '@demicodes/web-ui/transport/conversation-lane'
import { heldEdge, type ConversationReader } from '@demicodes/web-ui/transport/protocol'
import { apiRequest, apiUrl, readResponse } from '../api/client'
import {
  conversationFrameSchema,
  recentCostSchema,
  viewedConversationSchema,
  type ViewedConversation,
} from '../api/generated/web-api'
import type { Conversation } from '../state/types'
import { applyConversationEvent, describeAttachments } from '../conversation/activity'
import { followMetadata, newConversation } from '../conversation/entry'
import { createConversationHistoryReads, readLatestTranscript } from '../conversation/history'
import { costShown, createRecentCostWatch } from '../conversation/recent-cost'

/** What the model menu says of a model it has no name for: nothing, before the read. */
const UNNAMED: JobModel = { providerId: '', provider: '', model: '', effort: null, fast: false }

/** The routes of the view link `token`, the only ones its page reaches. */
export function viewPath(token: string): string {
  return `/views/${encodeURIComponent(token)}`
}

/** Reads what the view link `token` shows of its conversation. */
export async function readViewed(token: string, signal: AbortSignal): Promise<ViewedConversation> {
  return readResponse(await apiRequest(`${viewPath(token)}/conversation`, { signal }), viewedConversationSchema)
}

/**
 * The conversation the view link `token` shows, as its page follows it from
 * `viewed`, its first read: the conversation's record as the store holds one,
 * its client and runtime over the link's routes and its page connection, whose
 * lane only watches, and what its bar and cards read.
 *
 * `lost` is told when the connection could not be made, for the page to read
 * whether the link still opens.
 */
export function watchViewed(token: string, viewed: ViewedConversation, lost: () => void) {
  const base = viewPath(token)
  const lifetime = new AbortController()
  const conversation = reactive(newConversation(viewed.conversation.conversation)) as Conversation
  const runsOn = shallowRef(viewed.runsOn)
  const takeViewed = (read: ViewedConversation): void => {
    conversation.metadata.replace(read.conversation.metadata)
    runsOn.value = read.runsOn
  }
  takeViewed(viewed)
  // The conversation shows its metadata as it changes, as the store follows it.
  watch(() => conversation.metadata.shown.value, () => followMetadata(conversation), { immediate: true, flush: 'sync' })
  // A model changed meanwhile is named by a read again.
  watch(
    () => conversation.metadata.shown.value?.model,
    (model, before) => {
      if (before !== undefined && (model?.providerId !== before?.providerId || model?.modelId !== before?.modelId)) {
        readViewed(token, lifetime.signal).then(takeViewed, () => {})
      }
    },
  )

  let first: ViewedConversation | null = viewed
  const reader: ConversationReader = {
    conversation: async (signal, { whole }) => {
      const read = first ?? await readViewed(token, signal)
      first = null
      if (whole) {
        takeViewed(read)
      }
      const detail = read.conversation
      return { agents: detail.agents, entries: detail.entries, tags: whole ? { metadata: detail.metadataTag } : {} }
    },
    transcript: async (node, signal, options) => (await readLatestTranscript(base, node, signal, options?.first ?? false)).page,
  }

  configurePageConnection({ url: () => apiUrl(`${base}/connect`), failedToOpen: lost })
  const client = createConversationClient(conversation.id, {
    reader,
    conversationFrame: (frame: unknown) => {
      const parsed = conversationFrameSchema.safeParse(frame)
      if (!parsed.success) {
        return null
      }
      const data = parsed.data
      switch (data.type) {
        case 'conversation':
          return { apply: () => conversation.metadata.replace(data.metadata), tagged: { value: 'metadata', tag: data.etag } }
        case 'attachments':
          return { apply: () => describeAttachments(conversation, data.attachments) }
        // The lane that watches follows nothing else of the conversation's own.
        default:
          return { apply: () => {} }
      }
    },
    shows: (node) => node === null || node === conversation.id || node === conversation.shownSubagent,
  })
  const runtime = new ConversationRuntime({
    state: conversation,
    client,
    held: (node) => {
      const agent = conversation.subagents.find((each) => each.id === node)
      return agent ? heldEdge(agent.history) : undefined
    },
    shown: () => conversation.history.windows.length > 0,
    onEvent: (event) => applyConversationEvent(conversation, event),
  })
  void runtime.connect().catch(() => {})

  const history = createConversationHistoryReads(() => lifetime.signal, () => runtime.connected, () => null, () => base)

  /** Follows the child the Agents window shows, and lets the one before go. */
  function showSubagent(id: string | null): void {
    const before = conversation.shownSubagent
    if (before === id) {
      return
    }
    if (before !== null) {
      runtime.unsubscribeNode(before)
    }
    conversation.shownSubagent = id
    if (id !== null) {
      void runtime.subscribeNode(id).catch(() => {})
    }
  }

  const watchRecentCost = createRecentCostWatch({
    read: async (_conversation, window, signal) =>
      readResponse(await apiRequest(`${base}/cost/recent?window=${window}`, { signal }), recentCostSchema),
    failed: () => {},
  })

  /** What the root agent runs on and how far its turn got, which the bar says. */
  const run = computed<AgentRun>(() => {
    const blocks = latestBlocks(conversation.history)
    const asked = blocks.findLast((block) => block.type === 'user')
    const last = blocks.at(-1)
    const working = conversation.phase !== 'idle'
    const outcome = conversation.metadata.shown.value?.lastTurn?.outcome
    const phase: SubagentPhase = working ? 'running' : outcome === 'failed' ? 'error' : outcome === 'stopped' ? 'aborted' : 'completed'
    const endedAt = last && 'endedAt' in last && typeof last.endedAt === 'string' ? last.endedAt : last?.createdAt
    return {
      profile: null,
      model: runsOn.value ?? UNNAMED,
      round: 1,
      forkedFrom: null,
      phase,
      startedAt: asked?.createdAt ?? blocks[0]?.createdAt ?? new Date(0).toISOString(),
      ...(working || endedAt === undefined ? {} : { endedAt }),
      cwd: conversation.cwd,
      device: null,
    }
  })

  return {
    conversation,
    run,
    history,
    showSubagent,
    cost: (node: string | null = null) => costShown(conversation, node),
    watchRecentCost: (window: Parameters<typeof watchRecentCost>[1]) => watchRecentCost(conversation, window),
    dispose(): void {
      lifetime.abort()
      runtime.dispose()
    },
  }
}
