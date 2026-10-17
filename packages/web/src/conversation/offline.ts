import { toRaw, watch, type WatchStopHandle } from 'vue'
import { latestWindow } from '@demicodes/web-ui/transport/protocol'
import { transcriptShown } from '@demicodes/web-ui/agent/session-status'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { onConversationCopyDropped, writeConversationCopy, writeConversationOpened, type AgentCopy, type ConversationCopy } from '../state/offline'
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
 * page holds of it, once as the page first opens it and then at most every
 * `COPY_DELAY_MS` while it changes, and at once on `keep`, as the page
 * hides. Opening it again writes only when it was opened, never its copy,
 * unless the cache has dropped the copy meanwhile, which it then writes.
 */
export function createConversationCopies(items: () => Conversation[], userId: () => string | null) {
  /** When the user last opened each conversation on this page, which each write of its copy carries. */
  const openedAt = new Map<string, number>()
  /** The opened conversations whose copy the cache dropped since this page wrote it. */
  const dropped = new Set<string>()
  /** What stops hearing of dropped copies; none until a conversation is opened. */
  let stopDrops: (() => void) | null = null
  /** What watches each opened conversation for a change. */
  const watches = new Map<string, WatchStopHandle>()
  /** The conversations whose copy is to be written. */
  const due = new Set<string>()
  let timer: ReturnType<typeof setTimeout> | null = null

  function find(id: string): Conversation | undefined {
    return items().find((item) => item.id === id)
  }

  /** The user opened conversation `id`: its copy is kept from now on, and it was opened now. */
  function opened(id: string): void {
    openedAt.set(id, Date.now())
    stopDrops ??= onConversationCopyDropped((gone) => {
      if (watches.has(gone)) {
        dropped.add(gone)
      }
    })
    const user = userId()
    if (dropped.delete(id)) {
      changed(id)
    } else if (user && watches.has(id)) {
      // A failed write says so itself; the copy stays as it was.
      void writeConversationOpened(user, id, openedAt.get(id)!)
    }
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
      // What the page holds of it was never written by this page.
      changed(id)
    }
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
      const copy = conversation ? copyOf(conversation) : null
      if (user && copy) {
        dropped.delete(id)
        writes.push(writeConversationCopy(user, id, copy, openedAt.get(id) ?? Date.now()))
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
    stopDrops?.()
    stopDrops = null
    dropped.clear()
    openedAt.clear()
    due.clear()
  }

  return { opened, keep, clear }
}

/**
 * What the page holds of `conversation` as its copy: the latest window of
 * its root as held, each block whole or light as the page holds it, with
 * its agents; none while it shows no transcript, or holds no latest window.
 *
 * The copy is made of the data under the store's proxies (`raw`), which the
 * database's own copy of the record, the browser's structured clone, reads
 * as it is, so nothing walks the transcript to unwrap it. The transcript is
 * frozen, which the store never puts behind a proxy (`heldTranscript`); a
 * field of the conversation the store replaced with a spread of an older
 * value may hold a proxy for each entry it kept, so each entry of those is
 * unwrapped, one level, as the data under it is the page's own.
 */
function copyOf(conversation: Conversation): ConversationCopy | null {
  const raw = toRaw(conversation)
  if (!transcriptShown(raw.load)) {
    return null
  }
  const { history } = raw
  const window = latestWindow(history)
  if (!window && history.length > 0) {
    return null
  }
  // When each piece of a streamed text arrived only fades it as it arrives.
  const blocks = (window?.blocks ?? []).map((block) => (block.arrivals ? { ...block, arrivals: undefined } : block))
  const held = new Set(blocks.map((block) => block.id))
  return {
    transcript: { length: history.length, start: window?.start ?? 0, blocks },
    failures: Object.fromEntries(
      Object.entries(raw.failures).flatMap(([id, facts]) => (held.has(id) ? [[id, toRaw(facts)]] : [])),
    ),
    instructions: raw.instructions.map((entry) => toRaw(entry)),
    summaries: raw.summaries,
    agents: raw.subagents.map((agent) => agentCopy(toRaw(agent))),
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
