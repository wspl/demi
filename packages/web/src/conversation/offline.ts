import { toRaw, watch, type WatchStopHandle } from 'vue'
import type { AgentEntry, ProviderFailureFacts, Version } from '@demicodes/protocol'
import { latestWindow, windowEnd, type HeldBlock, type HeldTranscript, type TranscriptWindow } from '@demicodes/web-ui/transport/protocol'
import { transcriptShown } from '@demicodes/web-ui/agent/session-status'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { NO_COST } from '@demicodes/web-ui/agent/context-cost'
import { writeConversationCopy, writeNode, type ConversationCopy, type NodeWrite } from '../state/offline'
import type { Conversation, SideRecord } from '../state/types'

/**
 * How long the page lets a conversation's changes gather before it writes
 * them to the disk: a turn that streams changes the transcript many times a
 * second.
 */
export const COPY_DELAY_MS = 3_000

/** What the page last wrote of a node: each block by its index, the length and the runs. */
interface Written {
  blocks: Map<number, HeldBlock>
  length: number
}

/**
 * The disk copies of the conversations the user opens on this page
 * (`../state/offline.ts`): each opened conversation's metadata and agents,
 * and each of its nodes' blocks the page holds, written at most every
 * `COPY_DELAY_MS` while they change and at once on `keep`, as the page
 * hides. Only the blocks that changed since the last write go, each once it
 * is complete: a block still streaming waits for its end. A write's runs
 * join the ones the disk holds, which earlier pages and earlier visits
 * wrote, while the backend vouched for them: the page showed them from the
 * disk and its subscription named their edge. Otherwise, as when a reset
 * left stale blocks or the page never showed the disk's copy, the write's
 * runs replace the disk's.
 */
export function createConversationCopies(items: () => Conversation[], userId: () => string | null) {
  /** What watches each opened conversation for a change. */
  const watches = new Map<string, WatchStopHandle>()
  /** The conversations whose metadata and agents are to be written. */
  const due = new Set<string>()
  /** What the page last wrote of each node, by conversation and node. */
  const written = new Map<string, Map<string, Written>>()
  /** Each node's history as the page last looked for stale blocks in it. */
  const looked = new Map<string, HeldTranscript>()
  /** The nodes whose next write replaces the disk's runs, as a reset held stale blocks in them. */
  const unvouched = new Set<string>()
  let timer: ReturnType<typeof setTimeout> | null = null

  function find(id: string): Conversation | undefined {
    return items().find((item) => item.id === id)
  }

  /** The user opened conversation `id`: what the page holds of it is kept on disk from now on. */
  function opened(id: string): void {
    if (watches.has(id)) {
      return
    }
    watches.set(id, watch(
      () => {
        const conversation = find(id)
        return conversation && transcriptShown(conversation.load)
          ? [conversation.metadata.read.value, conversation.agentsHeld, conversation.history, conversation.failures,
              conversation.instructions, conversation.summaries,
              ...conversation.subagents.map((agent) => agent.history), ...conversation.sides.map((side) => side.history)]
          : null
      },
      (shown) => {
        const conversation = find(id)
        if (shown && conversation) {
          lookForStale(conversation)
          changed(id)
        }
      },
      { immediate: true },
    ))
  }

  /**
   * Marks the nodes of `conversation` whose history holds a stale block,
   * which a reset that could not vouch for it left there. It runs before
   * the page reads that block again, which waits for the backend.
   */
  function lookForStale(conversation: Conversation): void {
    const nodes: [string, HeldTranscript][] = [
      [conversation.id, conversation.history],
      ...conversation.subagents.map((agent): [string, HeldTranscript] => [agent.id, agent.history]),
      ...conversation.sides.map((side): [string, HeldTranscript] => [side.id, side.history]),
    ]
    for (const [node, history] of nodes) {
      if (looked.get(node) === history) {
        continue
      }
      looked.set(node, history)
      if (history.windows.some((window) => window.blocks.some((block) => block.stale))) {
        unvouched.add(node)
      }
    }
  }

  function changed(id: string): void {
    due.add(id)
    timer ??= setTimeout(() => void keep(), COPY_DELAY_MS)
  }

  /** Writes what is due now; resolves once it is written. */
  function keep(): Promise<void> {
    if (timer !== null) {
      clearTimeout(timer)
      timer = null
    }
    const user = userId()
    const writes: Promise<void>[] = []
    for (const id of due) {
      const conversation = find(id)
      if (!user || !conversation || !transcriptShown(conversation.load)) {
        continue
      }
      const raw = toRaw(conversation)
      const copy = copyOf(raw)
      if (copy) {
        writes.push(writeConversationCopy(user, id, copy))
      }
      const nodes = written.get(id) ?? new Map<string, Written>()
      written.set(id, nodes)
      const write = (node: string, history: HeldTranscript, failures: Record<string, ProviderFailureFacts>, active: boolean, root: boolean) => {
        // A node the page did not show from the disk holds nothing the
        // backend vouched for the disk's runs by, as a stale block says of one it did.
        const before = nodes.get(node)
        const change = nodeWrite(raw, history, failures, active, root, before, before === undefined || unvouched.has(node))
        if (change) {
          nodes.set(node, change.written)
          unvouched.delete(node)
          writes.push(writeNode(user, node, change.write))
        }
      }
      write(id, raw.history, raw.failures, raw.phase !== 'idle', true)
      for (const agent of raw.subagents) {
        write(agent.id, agent.history, agent.failures, agent.sessionPhase !== 'idle', false)
      }
      for (const side of raw.sides) {
        write(side.id, side.history, side.failures, side.sessionPhase !== 'idle', false)
      }
    }
    due.clear()
    return Promise.all(writes).then(() => {})
  }

  /**
   * The page shows `history` of node `node` of conversation `id` as the
   * disk keeps it: those blocks are written already, and only what changes
   * of them is written again.
   */
  function shownFromDisk(id: string, node: string, history: HeldTranscript): void {
    const nodes = written.get(id) ?? new Map<string, Written>()
    written.set(id, nodes)
    const blocks = new Map<number, HeldBlock>()
    for (const window of history.windows) {
      window.blocks.forEach((block, offset) => blocks.set(window.start + offset, block))
    }
    nodes.set(node, { blocks, length: history.length })
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
    written.clear()
    looked.clear()
    unvouched.clear()
    due.clear()
  }

  /** Forgets conversation `id`, which the page let go: what it wrote stays on disk. */
  function released(id: string): void {
    watches.get(id)?.()
    watches.delete(id)
    for (const node of written.get(id)?.keys() ?? []) {
      looked.delete(node)
      unvouched.delete(node)
    }
    written.delete(id)
    due.delete(id)
  }

  return { opened, shownFromDisk, keep, clear, released }
}

/**
 * What changed of a node's transcript since `before`, as the disk keeps it;
 * none when nothing did, or the page holds none of it.
 *
 * Every window the page holds is a run, but the latest stops before its
 * first block that is not complete: the last block while the node works,
 * and a call still executing. Such a run says no version, which the next
 * subscription then reads from the stream again. A stale block is no
 * block's whole copy, so the runs leave it out; `replace` says the runs
 * replace the disk's.
 */
function nodeWrite(
  conversation: Conversation,
  history: HeldTranscript,
  failures: Record<string, ProviderFailureFacts>,
  active: boolean,
  root: boolean,
  before: Written | undefined,
  replace: boolean,
): { write: NodeWrite, written: Written } | null {
  if (history.windows.length === 0) {
    return null
  }
  const latest = latestWindow(history)
  let length = history.length
  let version: Version | undefined = history.version
  let complete = Infinity
  if (latest) {
    const pending = latest.blocks.findIndex((block, offset) =>
      (block.type === 'tool_call' && block.status === 'executing') || (active && offset === latest.blocks.length - 1))
    if (pending >= 0) {
      length = latest.start + pending
      version = undefined
      complete = length
    }
  }
  const runs = history.windows.flatMap((window) => freshRuns(window, complete))
  const blocks: NodeWrite['blocks'] = []
  const next = new Map<number, HeldBlock>()
  for (const run of runs) {
    const window = history.windows.find((each) => each.start <= run.start && run.start < windowEnd(each))!
    for (let index = run.start; index < run.end; index += 1) {
      const block = window.blocks[index - window.start]!
      next.set(index, block)
      if (before?.blocks.get(index) !== block) {
        // When each piece of a streamed text arrived only fades it as it arrives.
        const plain = block.arrivals ? { ...block, arrivals: undefined } : block
        const failure = failures[block.id]
        blocks.push({ index, block: plain, ...(failure ? { failure: toRaw(failure) } : {}) })
      }
    }
  }
  const cut = before !== undefined && length < before.length
  if (blocks.length === 0 && !cut && !replace && before?.length === length) {
    return null
  }
  return {
    write: {
      conversation: conversation.id,
      length,
      ...(version ? { version } : {}),
      runs: runs.filter((run) => run.end > run.start),
      instructions: root ? conversation.instructions.map((entry) => toRaw(entry)) : [],
      summaries: root ? { ...conversation.summaries } : {},
      blocks,
      ...(cut ? { cutFrom: length } : {}),
      ...(replace ? { replaceRuns: true as const } : {}),
    },
    written: { blocks: next, length },
  }
}

/** The runs of `window`'s blocks that are not stale, before the index `end`. */
function freshRuns(window: TranscriptWindow, end: number): { start: number, end: number }[] {
  const runs: { start: number, end: number }[] = []
  window.blocks.forEach((block, offset) => {
    const index = window.start + offset
    if (block.stale || index >= end) {
      return
    }
    const last = runs.at(-1)
    if (last?.end === index) {
      last.end = index + 1
    } else {
      runs.push({ start: index, end: index + 1 })
    }
  })
  return runs
}

/**
 * The conversation's metadata and agents as the disk keeps them: what lives
 * only while the conversation runs, its waits and the calls being written,
 * stays out. None before the page holds its metadata and agents.
 */
function copyOf(conversation: Conversation): ConversationCopy | null {
  const metadata = conversation.metadata.read.value
  const version = conversation.agentsHeld
  if (!metadata || !version) {
    return null
  }
  const entries: AgentEntry[] = [
    {
      id: conversation.id,
      role: { kind: 'root' },
      phase: conversation.phase,
      waitedMs: null,
      pendingCalls: [],
      context: toRaw(conversation.contextUsage),
      cost: toRaw(conversation.cost),
      // The waiting inputs live while the session does and are never kept.
      queue: [],
      pendingSteers: [],
      starting: null,
    },
    ...conversation.subagents.map((agent) => subagentEntry(toRaw(agent))),
    ...conversation.sides.flatMap((side) => sideEntry(toRaw(side))),
  ]
  return { metadata: toRaw(metadata), agents: { version: toRaw(version), entries } }
}

/** A child as its entry says it. */
function subagentEntry(agent: SubagentRecord): AgentEntry {
  return {
    id: agent.id,
    role: {
      kind: 'subagent',
      number: agent.number,
      parent: agent.parent,
      description: agent.name,
      profile: agent.profile,
      model: toRaw(agent.model),
      round: agent.round,
      forkedFrom: agent.forkedFrom,
      state: agent.phase,
      startedAt: agent.startedAt,
      endedAt: agent.endedAt ?? null,
      cwd: agent.cwd,
      onPath: agent.onPath,
    },
    phase: agent.sessionPhase,
    waitedMs: null,
    pendingCalls: [],
    context: toRaw(agent.contextUsage),
    cost: toRaw(agent.cost),
    queue: [],
    pendingSteers: [],
    starting: null,
  }
}

/** A side question as its entry says it; none for one still being asked, which no entry names yet. */
function sideEntry(side: SideRecord): AgentEntry[] {
  if (!side.listed) {
    return []
  }
  const { id: _id, ...role } = toRaw(side.record)
  return [{
    id: side.id,
    role: { kind: 'side', ...role },
    phase: side.sessionPhase,
    waitedMs: null,
    pendingCalls: [],
    context: null,
    cost: NO_COST,
    queue: [],
    pendingSteers: [],
    starting: null,
  }]
}
