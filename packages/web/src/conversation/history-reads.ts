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
import { pageBeside, shownWindow } from '@demicodes/web-ui/agent/history'
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

/** Where a page is read from. */
type PageAt =
  | { kind: 'latest' }
  | { kind: 'before' | 'after'; index: number; edge: string }
  | { kind: 'around'; block: string }

/** A subagent or a side question whose transcript a holder keeps, by its node. */
export interface HistoryAgent {
  id: string
  history: HeldTranscript
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

  /** One page of the agent `node`'s transcript, the root's for null. */
  async function readPage(holder: T, node: string | null, at: PageAt): Promise<TranscriptPage> {
    const query = new URLSearchParams()
    if (at.kind === 'before' || at.kind === 'after') {
      query.set(at.kind, String(at.index))
      query.set('edge', at.edge)
    } else if (at.kind === 'around') {
      query.set('around', at.block)
    }
    const response = await apiRequest(`${path(holder, node)}/transcript?${query}`, { signal: signal() })
    return readResponse(response, transcriptPageSchema)
  }

  function agentOf(holder: T, node: string): HistoryAgent | undefined {
    return holder.subagents.find((agent) => agent.id === node) ?? holder.sides?.find((side) => side.id === node)
  }

  function historyOf(holder: T, node: string | null): HeldTranscript {
    return node === null ? holder.history : agentOf(holder, node)?.history ?? EMPTY_TRANSCRIPT
  }

  /** Takes `page` into the agent's history, with what it names beside its blocks. */
  function take(holder: T, node: string | null, page: TranscriptPage): void {
    const failures = { ...page.failures }
    if (node === null) {
      holder.history = addPage(holder.history, page, live(holder, null))
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
      agent.history = addPage(agent.history, page, live(holder, node))
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
    return once(`${holder.id}:${node ?? ''}:${JSON.stringify(at)}`, async () => {
      const disk = at.kind === 'before' ? await kept?.(holder, node, at.index, at.edge) : null
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
      take(holder, node, await readPage(holder, node, at))
    })
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
    const window = node === null ? shownWindow(history, holder.shownAt) : shownWindow(history, null)
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
      setHistory(holder, node, latestOnly(historyOf(holder, node)))
      if (node === null) {
        holder.shownAt = null
      }
    }
  }

  function setHistory(holder: T, node: string | null, history: HeldTranscript): void {
    if (node === null) {
      holder.history = history
      return
    }
    const agent = agentOf(holder, node)
    if (agent) {
      agent.history = history
    }
  }

  async function recover(holder: T, node: string | null, anchor: string): Promise<void> {
    setHistory(holder, node, latestOnly(historyOf(holder, node)))
    await readAround(holder, node, anchor).catch((error: unknown) => {
      // The rewrite removed the block: the latest is what the reader sees now.
      if (!(error instanceof ApiError) || error.code !== 'block_not_found') {
        throw error
      }
      if (node === null) {
        holder.shownAt = null
      }
    })
  }

  /**
   * Shows the window around the block `block`, reading it first when the
   * page does not hold it. A subagent's panel shows one window, from its
   * latest back, so there its latest page is read first, as the panel reads
   * it, and the pages before it until one holds the block.
   */
  async function readAround(holder: T, node: string | null, block: string): Promise<void> {
    const held = () => historyOf(holder, node).windows.some((window) => window.blocks.some((candidate) => candidate.id === block && !candidate.stale))
    if (node === null) {
      if (!held()) {
        await read(holder, node, { kind: 'around', block })
      }
      holder.shownAt = block
      return
    }
    if (historyOf(holder, node).windows.length === 0) {
      await read(holder, node, { kind: 'latest' })
    }
    let start = shownWindow(historyOf(holder, node), null).start
    while (!held() && start > 0) {
      await readBeside(holder, node, 'before')
      const reached = shownWindow(historyOf(holder, node), null).start
      // A rewrite moved the window: the next scroll there reads on.
      if (reached >= start) {
        return
      }
      start = reached
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

