import {
  EMPTY_TRANSCRIPT,
  addPage,
  addWindow,
  heldTranscript,
  latestWindow,
  withWholeBlock,
  type HeldBlock,
  type HeldTranscript,
  type TranscriptWindow,
} from '@demicodes/web-ui/transport/protocol'
import { boundedHistory, pageBeside, shownWindow } from '@demicodes/web-ui/agent/history'
import { oneRunPerKey } from '@demicodes/utils'
import { ApiError, apiRequest, readResponse } from '../api/client'
import {
  transcriptPageSchema,
  wholeBlockSchema,
  type TranscriptPage,
} from '../api/generated/web-api'
import type { Block, InstructionEntry, ProviderFailureFacts } from '@demicodes/protocol'

/** The blocks a disk copy holds whole, with the facts of their failures. */
export interface KeptBlocks {
  window: TranscriptWindow<HeldBlock>
  failures: Record<string, ProviderFailureFacts>
}

/** The latest window of `history` alone, from after its last stale block. */
function latestOnly(history: HeldTranscript): HeldTranscript {
  const latest = latestWindow(history)
  if (!latest) {
    return heldTranscript(history.length, [], history.version)
  }
  const fresh = latest.blocks.findLastIndex((block) => block.stale) + 1
  const blocks = latest.blocks.slice(fresh)
  return heldTranscript(history.length, blocks.length > 0 ? [{ start: latest.start + fresh, blocks }] : [], history.version)
}

/** How long the reads a view no longer needs run on, for a view that asks for them again. */
const RELEASE_DELAY_MS = 500

/** Where a page is read from. */
type PageAt =
  | { kind: 'latest' }
  | { kind: 'before' | 'after'; index: number; edge: string }
  | { kind: 'around'; block: string }

/** A subagent or a side question whose transcript a holder keeps, by its node. */
export interface HistoryAgent {
  id: string
  history: HeldTranscript
  /** The block its view shows, which bounds what of its history the page holds; null for the latest. */
  shownAt: string | null
  failures: Record<string, ProviderFailureFacts>
}

/**
 * What holds an agent tree's transcripts as the page reads them: a
 * conversation, or a share's snapshot, which its shared page reads through
 * the share's routes the same way.
 */
export interface HistoryHolder {
  id: string
  history: HeldTranscript
  failures: Record<string, ProviderFailureFacts>
  instructions: InstructionEntry[]
  summaries: Record<string, number>
  shownAt: string | null
  subagents: HistoryAgent[]
  /** A conversation's side questions, whose transcripts its routes read by their nodes as a subagent's; a share has none. */
  sides?: HistoryAgent[]
}

/**
 * The reads of a transcript's history, each held where the page shows it, from the
 * routes under `path`, the agent `node`'s (the root's for null) `…/agents/:node`,
 * whose `transcript` reads its pages and `blocks/:block` a block whole. A page that
 * one read is already fetching is not fetched twice: a second read waits for the
 * first; `live` says whether a stream keeps the end of the agent `node`'s
 * transcript, whose length then stands. `kept` reads the blocks a disk copy
 * holds before an index whose block it holds as the edge, which a page before
 * a window takes without asking the backend.
 */
export function createHistoryReads<T extends HistoryHolder>(
  signal: () => AbortSignal,
  live: (holder: T, node: string | null) => boolean,
  path: (holder: T, node: string | null) => string,
  kept?: (holder: T, node: string | null, end: number, edge: string) => Promise<KeptBlocks | null>,
) {
  const once = oneRunPerKey<string, void>()

  /**
   * The reads of each agent's transcript under way, by holder and node: what
   * stops them, and the pages they read. `release` gives an agent a new
   * group, and stops the old one shortly after, unless a view asks again for
   * a page it still reads meanwhile, as Telegram's clients cancel the read of
   * a hole no view needs
   * (`docs/decisions/0025-a-transcript-is-shown-from-a-bounded-window.md`).
   */
  interface ReadGroup {
    controller: AbortController
    keys: Set<string>
    stop?: ReturnType<typeof setTimeout>
  }
  const groups = new Map<string, ReadGroup>()
  const releasing = new Set<ReadGroup>()

  function groupKey(holder: T, node: string | null): string {
    return `${holder.id}:${node ?? ''}`
  }

  /** The group a read of `key` joins: one stopping that still reads it, kept, or the agent's own. */
  function groupFor(holder: T, node: string | null, key: string): ReadGroup {
    const at = groupKey(holder, node)
    for (const group of releasing) {
      if (group.keys.has(key)) {
        clearTimeout(group.stop)
        releasing.delete(group)
        // The agent's own again, unless a newer group took its place, so
        // the next release stops it too.
        if (!groups.has(at)) {
          groups.set(at, group)
        }
        return group
      }
    }
    let group = groups.get(at)
    if (!group) {
      group = { controller: new AbortController(), keys: new Set() }
      groups.set(at, group)
    }
    return group
  }

  /**
   * The views of `holder`'s agent `node`, or of all its agents for undefined,
   * no longer need the reads under way: they stop shortly, unless asked for
   * again meanwhile.
   */
  function release(holder: T, node?: string | null): void {
    for (const [at, group] of groups) {
      if (node === undefined ? !at.startsWith(`${holder.id}:`) : at !== groupKey(holder, node)) {
        continue
      }
      groups.delete(at)
      if (group.keys.size === 0) {
        continue
      }
      releasing.add(group)
      group.stop = setTimeout(() => {
        releasing.delete(group)
        group.controller.abort()
      }, RELEASE_DELAY_MS)
    }
  }

  /** One page of the agent `node`'s transcript, the root's for null. */
  async function readPage(holder: T, node: string | null, at: PageAt, stop: AbortSignal): Promise<TranscriptPage> {
    const query = new URLSearchParams()
    if (at.kind === 'before' || at.kind === 'after') {
      query.set(at.kind, String(at.index))
      query.set('edge', at.edge)
    } else if (at.kind === 'around') {
      query.set('around', at.block)
    }
    const response = await apiRequest(`${path(holder, node)}/transcript?${query}`, { signal: AbortSignal.any([signal(), stop]) })
    return readResponse(response, transcriptPageSchema)
  }

  function agentOf(holder: T, node: string): HistoryAgent | undefined {
    return holder.subagents.find((agent) => agent.id === node) ?? holder.sides?.find((side) => side.id === node)
  }

  function historyOf(holder: T, node: string | null): HeldTranscript {
    return node === null ? holder.history : agentOf(holder, node)?.history ?? EMPTY_TRANSCRIPT
  }

  /** The block the agent `node`'s view shows, the root's for null; null for the latest. */
  function shownAtOf(holder: T, node: string | null): string | null {
    return node === null ? holder.shownAt : agentOf(holder, node)?.shownAt ?? null
  }

  /**
   * Shows the block `block` in the agent `node`'s view, or the latest for
   * null: what of its history the page holds follows it
   * (`docs/decisions/0025-a-transcript-is-shown-from-a-bounded-window.md`).
   */
  function showAt(holder: T, node: string | null, block: string | null): void {
    const before = shownWindow(historyOf(holder, node), shownAtOf(holder, node))
    // A move to another part of the transcript leaves the reads at the
    // part shown before to a view that asks for them again.
    if (block === null ? before !== latestWindow(historyOf(holder, node)) : !before.blocks.some((each) => each.id === block)) {
      release(holder, node)
    }
    if (node === null) {
      holder.shownAt = block
    } else {
      const agent = agentOf(holder, node)
      if (!agent) {
        return
      }
      agent.shownAt = block
    }
    setHistory(holder, node, historyOf(holder, node))
  }

  /** Takes `page` into the agent's history, with what it names beside its blocks. */
  function take(holder: T, node: string | null, page: TranscriptPage): void {
    const failures = { ...page.failures }
    if (node === null) {
      holder.history = boundedHistory(addPage(holder.history, page, live(holder, null)), holder.shownAt)
      holder.failures = { ...holder.failures, ...failures }
      holder.instructions = page.instructions
      holder.summaries = {
        ...holder.summaries,
        ...Object.fromEntries(page.summaries.map((summary) => [summary.marker, summary.summaryTokens])),
      }
      return
    }
    const agent = agentOf(holder, node)
    if (agent) {
      agent.history = boundedHistory(addPage(agent.history, page, live(holder, node)), agent.shownAt)
      agent.failures = { ...agent.failures, ...failures }
    }
  }

  /**
   * Reads a page once at a time, and takes it: the page before a window from
   * the disk copy when it holds it there. The disk's run must hold the
   * window's first block at its index, so a rewrite, which gives every block
   * from where it starts a new id, never reads the old blocks back.
   */
  function read(holder: T, node: string | null, at: PageAt): Promise<void> {
    const key = `${holder.id}:${node ?? ''}:${JSON.stringify(at)}`
    const group = groupFor(holder, node, key)
    return once(key, async () => {
      group.keys.add(key)
      try {
        await readInto(holder, node, at, group.controller.signal)
      } finally {
        group.keys.delete(key)
      }
    })
  }

  /** Reads the page `at` and takes it, unless `stop` stops it first. */
  async function readInto(holder: T, node: string | null, at: PageAt, stop: AbortSignal): Promise<void> {
    const disk = at.kind === 'before' ? await kept?.(holder, node, at.index, at.edge) : null
    stop.throwIfAborted()
    if (disk && disk.window.blocks.length > 0) {
      const history = historyOf(holder, node)
      setHistory(holder, node, addWindow(history, { start: disk.window.start, length: history.length, blocks: disk.window.blocks }, true))
      if (node === null) {
        holder.failures = { ...holder.failures, ...disk.failures }
      } else {
        const agent = agentOf(holder, node)
        if (agent) {
          agent.failures = { ...agent.failures, ...disk.failures }
        }
      }
      return
    }
    take(holder, node, await readPage(holder, node, at, stop))
  }

  /**
   * Reads the next page at an edge of the window the reader is in. A page the
   * transcript's rewrite refuses drops the windows that no longer join and
   * reads the page around the block in view again.
   *
   * Before reading further back, the window's stale blocks are read again:
   * the page before the latest of them that the reset vouched for, or around
   * the latest when none follows it in the window.
   */
  async function readBeside(holder: T, node: string | null, side: 'before' | 'after'): Promise<void> {
    const history = historyOf(holder, node)
    const window = shownWindow(history, shownAtOf(holder, node))
    if (side === 'before' && window.blocks.some((block) => block.stale)) {
      await readStale(holder, node, window)
      return
    }
    const beside = pageBeside(history, window, side)
    if (!beside) {
      return
    }
    try {
      await read(holder, node, { kind: side, ...beside })
    } catch (error) {
      if (!(error instanceof ApiError) || error.code !== 'transcript_changed') {
        throw error
      }
      await recover(holder, node, beside.edge)
    }
  }

  /** Reads the stale blocks of `window` again, as a page before the latest block after them. */
  async function readStale(holder: T, node: string | null, window: TranscriptWindow<HeldBlock>): Promise<void> {
    const latest = window.blocks.findLastIndex((block) => block.stale)
    const next = window.blocks[latest + 1]
    const at: PageAt = next
      ? { kind: 'before', index: window.start + latest + 1, edge: next.id }
      : { kind: 'around', block: window.blocks[latest]!.id }
    try {
      await read(holder, node, at)
    } catch (error) {
      if (!(error instanceof ApiError) || (error.code !== 'transcript_changed' && error.code !== 'block_not_found')) {
        throw error
      }
      // The transcript changed again, or no longer holds the block: what is
      // stale is no longer there to read, and the reader sees the latest.
      showAt(holder, node, null)
      setHistory(holder, node, latestOnly(historyOf(holder, node)))
    }
  }

  /** Sets the agent's history, bounded to what its view shows. */
  function setHistory(holder: T, node: string | null, history: HeldTranscript): void {
    if (node === null) {
      holder.history = boundedHistory(history, holder.shownAt)
      return
    }
    const agent = agentOf(holder, node)
    if (agent) {
      agent.history = boundedHistory(history, agent.shownAt)
    }
  }

  async function recover(holder: T, node: string | null, anchor: string): Promise<void> {
    setHistory(holder, node, latestOnly(historyOf(holder, node)))
    await readAround(holder, node, anchor).catch((error: unknown) => {
      // The rewrite removed the block: the latest is what the reader sees now.
      if (!(error instanceof ApiError) || error.code !== 'block_not_found') {
        throw error
      }
      showAt(holder, node, null)
    })
  }

  /**
   * Shows the window around the block `block`, reading it first when the
   * page does not hold it, for any agent of the tree: the view shows the block
   * first, so what the page holds follows it, then the page around it comes
   * and joins the other parts it meets.
   */
  async function readAround(holder: T, node: string | null, block: string): Promise<void> {
    const held = () => historyOf(holder, node).windows.some((window) => window.blocks.some((candidate) => candidate.id === block && !candidate.stale))
    showAt(holder, node, block)
    if (!held()) {
      await read(holder, node, { kind: 'around', block })
    }
  }

  return {
    /** The latest page of the agent `node`'s transcript. */
    readLatest: (holder: T, node: string | null) => read(holder, node, { kind: 'latest' }),
    /** The page before the window the reader is in. */
    readBefore: (holder: T, node: string | null) => readBeside(holder, node, 'before'),
    /** The page after the window the reader is in. */
    readAfter: (holder: T, node: string | null) => readBeside(holder, node, 'after'),
    readAround,
    showAt,
    release,
    /**
     * Reads a block whole, for a row the page holds light: a block the page holds is
     * replaced by it, and the block is the answer.
     */
    async readWhole(holder: T, node: string | null, id: string): Promise<Block> {
      const response = await apiRequest(`${path(holder, node)}/blocks/${encodeURIComponent(id)}`, { signal: signal() })
      const { block, failures } = await readResponse(response, wholeBlockSchema)
      if (node === null) {
        holder.history = withWholeBlock(holder.history, block)
        holder.failures = { ...holder.failures, ...failures }
        return block
      }
      const agent = agentOf(holder, node)
      if (agent) {
        agent.history = withWholeBlock(agent.history, block)
        agent.failures = { ...agent.failures, ...failures }
      }
      return block
    }
  }
}

