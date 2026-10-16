import { watch, type WatchStopHandle } from 'vue'
import { latestWindow } from '@demicodes/web-ui/transport/protocol'
import { transcriptShown } from '@demicodes/web-ui/agent/session-status'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { writeConversationCopy, type AgentCopy, type ConversationCopy } from '../state/offline'
import type { Conversation } from '../state/types'

/**
 * How long the page lets a conversation's changes gather before it writes
 * its copy for opening offline: a turn that streams changes the transcript
 * many times a second (`web-application.md` § Installing and opening offline).
 */
export const COPY_DELAY_MS = 3_000

/**
 * The copies of the conversations the user opens on this page, for opening
 * offline: each opened conversation's copy is replaced whole with what the
 * page holds of it, at most every `COPY_DELAY_MS` while it changes, and at
 * once on `keep`, as the page hides.
 */
export function createConversationCopies(items: () => Conversation[], userId: () => string | null) {
  /** When the user last opened each conversation on this page. */
  const openedAt = new Map<string, number>()
  /** What watches each opened conversation for a change. */
  const watches = new Map<string, WatchStopHandle>()
  /** The conversations whose copy is to be written. */
  const due = new Set<string>()
  let timer: ReturnType<typeof setTimeout> | null = null

  function find(id: string): Conversation | undefined {
    return items().find((item) => item.id === id)
  }

  /** The user opened conversation `id`: its copy is kept from now on, and says it was opened now. */
  function opened(id: string): void {
    openedAt.set(id, Date.now())
    if (!watches.has(id)) {
      watches.set(id, watch(
        () => {
          const conversation = find(id)
          return conversation
            ? [conversation.load, conversation.history, conversation.instructions, conversation.summaries,
                conversation.failures, conversation.subagents.map(agentCopy)]
            : null
        },
        () => changed(id),
      ))
    }
    changed(id)
  }

  function changed(id: string): void {
    due.add(id)
    timer ??= setTimeout(() => void keep(), COPY_DELAY_MS)
  }

  /** Writes the copies due now; resolves once they are written. */
  function keep(): Promise<void> {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    const user = userId()
    const writes: Promise<void>[] = []
    for (const id of due) {
      const conversation = find(id)
      const copy = conversation ? copyOf(conversation, openedAt.get(id) ?? Date.now()) : null
      if (user && copy) {
        writes.push(writeConversationCopy(user, id, copy))
      }
    }
    due.clear()
    return Promise.all(writes).then(() => {})
  }

  /** Forgets every conversation, as the page's account changes; nothing more is written. */
  function clear(): void {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    for (const stop of watches.values()) {
      stop()
    }
    watches.clear()
    openedAt.clear()
    due.clear()
  }

  return { opened, keep, clear }
}

/**
 * What the page holds of `conversation` as its copy: the latest window of
 * its root as held, each block whole or light as the page holds it, with
 * its agents; none while it shows no transcript, or holds no latest window.
 */
function copyOf(conversation: Conversation, openedAt: number): ConversationCopy | null {
  if (!transcriptShown(conversation.load)) {
    return null
  }
  const { history } = conversation
  const window = latestWindow(history)
  if (!window && history.length > 0) {
    return null
  }
  // When each piece of a streamed text arrived only fades it as it arrives.
  const blocks = (window?.blocks ?? []).map((block) => (block.arrivals ? { ...block, arrivals: undefined } : block))
  const held = new Set(blocks.map((block) => block.id))
  return {
    openedAt,
    transcript: { length: history.length, start: window?.start ?? 0, blocks },
    failures: Object.fromEntries(Object.entries(conversation.failures).filter(([id]) => held.has(id))),
    instructions: conversation.instructions,
    summaries: conversation.summaries,
    agents: conversation.subagents.map(agentCopy),
  }
}

/** A child as its job says it, which `subagentJobFields` turns back into its record. */
function agentCopy(agent: SubagentRecord): AgentCopy {
  return {
    subagentId: agent.id,
    number: agent.number,
    description: agent.name,
    profile: agent.profile,
    model: agent.model,
    round: agent.round,
    forkedFrom: agent.forkedFrom,
    phase: agent.phase,
    startedAt: agent.startedAt,
    endedAt: agent.endedAt ?? null,
    cwd: agent.cwd,
    onPath: agent.onPath,
  }
}
