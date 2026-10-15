import {
  EMPTY_TRANSCRIPT,
  addPage,
  latestWindow,
  windowEnd,
  withWholeBlock,
  type HeldTranscript,
} from '@demicodes/web-ui/transport/protocol'
import { shownWindow, type TranscriptReveal } from '@demicodes/web-ui/agent/history'
import { ApiError, apiRequest, readResponse } from '../api/client'
import {
  commandCallSchema,
  transcriptPageSchema,
  wholeBlockSchema,
  type TranscriptPage,
} from '../api/generated/web-api'
import type { Block, InstructionEntry, ProviderFailureFacts } from '@demicodes/protocol'
import type { Conversation } from '../state/types'

/** Where a page is read from (`web-api.md` § Pages). */
type PageAt =
  | { kind: 'latest' }
  | { kind: 'before' | 'after'; index: number; edge: string }
  | { kind: 'around'; block: string }

/** A subagent whose transcript a holder keeps, by the name its routes' `node` takes. */
export interface HistoryAgent {
  id: string
  history: HeldTranscript
  failures: Record<string, ProviderFailureFacts>
}

/**
 * What holds an agent tree's transcripts as the page reads them: a
 * conversation, or a share's snapshot, which its shared page reads through
 * the share's routes the same way (`sharing.md` § Storage and routes).
 */
export interface HistoryHolder {
  id: string
  history: HeldTranscript
  failures: Record<string, ProviderFailureFacts>
  instructions: InstructionEntry[]
  summaries: Record<string, number>
  shownAt: string | null
  subagents: HistoryAgent[]
}

/**
 * The reads of a transcript's history (`web-api.md` § Pages), each held
 * where the page shows it (`web-application.md` § Transcript windows), from
 * the routes under `path`, a conversation's or a share's `…/transcript`. A
 * page that one read is already fetching is not fetched twice: a second
 * read waits for the first; `live` says whether a stream keeps the end of
 * the agent `node`'s transcript, whose length then stands.
 */
export function createHistoryReads<T extends HistoryHolder>(
  signal: () => AbortSignal,
  live: (holder: T, node: string | null) => boolean,
  path: (holder: T) => string,
) {
  const fetching = new Map<string, Promise<void>>()

  /** One page of the agent `node`'s transcript, the root's for null. */
  async function readPage(holder: T, node: string | null, at: PageAt): Promise<TranscriptPage> {
    const query = new URLSearchParams()
    if (node !== null) {
      query.set('node', node)
    }
    if (at.kind === 'before' || at.kind === 'after') {
      query.set(at.kind, String(at.index))
      query.set('edge', at.edge)
    } else if (at.kind === 'around') {
      query.set('around', at.block)
    }
    const response = await apiRequest(`${path(holder)}?${query}`, { signal: signal() })
    return readResponse(response, transcriptPageSchema)
  }

  function historyOf(holder: T, node: string | null): HeldTranscript {
    return node === null
      ? holder.history
      : holder.subagents.find((agent) => agent.id === node)?.history ?? EMPTY_TRANSCRIPT
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
    const agent = holder.subagents.find((candidate) => candidate.id === node)
    if (agent) {
      agent.history = addPage(agent.history, page, live(holder, node))
      agent.failures = { ...agent.failures, ...failures }
    }
  }

  /** Reads a page once at a time, and takes it. */
  function read(holder: T, node: string | null, at: PageAt): Promise<void> {
    const key = `${holder.id}:${node ?? ''}:${JSON.stringify(at)}`
    const pending = fetching.get(key)
    if (pending) {
      return pending
    }
    const reading = (async () => {
      try {
        take(holder, node, await readPage(holder, node, at))
      } finally {
        fetching.delete(key)
      }
    })()
    fetching.set(key, reading)
    return reading
  }

  /**
   * Reads the next page at an edge of the window the reader is in
   * (`web-application.md` § Transcript windows). A page the transcript's
   * rewrite refuses drops the windows that no longer join and reads the
   * page around the block in view again (§ Rewrites).
   */
  async function readBeside(holder: T, node: string | null, side: 'before' | 'after'): Promise<void> {
    const history = historyOf(holder, node)
    const window = node === null ? shownWindow(history, holder.shownAt) : shownWindow(history, null)
    const edge = side === 'before' ? window.blocks[0] : window.blocks.at(-1)
    if (!edge || (side === 'before' ? window.start === 0 : windowEnd(window) >= history.length)) {
      return
    }
    const index = side === 'before' ? window.start : windowEnd(window) - 1
    try {
      await read(holder, node, { kind: side, index, edge: edge.id })
    } catch (error) {
      if (!(error instanceof ApiError) || error.code !== 'transcript_changed') {
        throw error
      }
      await recover(holder, node, edge.id)
    }
  }

  async function recover(holder: T, node: string | null, anchor: string): Promise<void> {
    const keep = (history: HeldTranscript): HeldTranscript => {
      const latest = latestWindow(history)
      return { length: history.length, windows: latest ? [latest] : [] }
    }
    if (node === null) {
      holder.history = keep(holder.history)
    } else {
      const agent = holder.subagents.find((candidate) => candidate.id === node)
      if (agent) {
        agent.history = keep(agent.history)
      }
    }
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
    const held = () => historyOf(holder, node).windows.some((window) => window.blocks.some((candidate) => candidate.id === block))
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
     * Reads a block whole, for a row the page holds light or a command's call
     * (`web-api.md` § Light form): a block the page holds is replaced by it,
     * and the block is the answer.
     */
    async readWhole(holder: T, node: string | null, id: string): Promise<Block> {
      const query = node === null ? '' : `?node=${encodeURIComponent(node)}`
      const response = await apiRequest(`${path(holder)}/blocks/${encodeURIComponent(id)}${query}`, { signal: signal() })
      const { block, failures } = await readResponse(response, wholeBlockSchema)
      if (node === null) {
        holder.history = withWholeBlock(holder.history, block)
        holder.failures = { ...holder.failures, ...failures }
        return block
      }
      const agent = holder.subagents.find((candidate) => candidate.id === node)
      if (agent) {
        agent.history = withWholeBlock(agent.history, block)
        agent.failures = { ...agent.failures, ...failures }
      }
      return block
    }
  }
}

/**
 * The reads of conversations' history (`web-api.md` § Conversation
 * history): a conversation's root is live while its stream is open, as
 * `live` says, and a subagent while it runs.
 */
export function createConversationHistoryReads(signal: () => AbortSignal, live: (conversation: Conversation) => boolean) {
  const reads = createHistoryReads<Conversation>(
    signal,
    (conversation, node) => node === null
      ? live(conversation)
      : conversation.subagents.find((agent) => agent.id === node)?.phase === 'running',
    (conversation) => `/conversations/${encodeURIComponent(conversation.id)}/transcript`,
  )
  return {
    ...reads,
    /** Where the call that started the command `commandId` lies (`web-api.md` § Subagents and commands). */
    async findCommand(conversation: Conversation, commandId: string): Promise<TranscriptReveal> {
      const response = await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/commands/${encodeURIComponent(commandId)}`,
        { signal: signal() },
      )
      const call = await readResponse(response, commandCallSchema)
      return { node: call.subagentId, blockId: call.blockId }
    },
  }
}

export type HistoryReads = ReturnType<typeof createConversationHistoryReads>
