import {
  EMPTY_TRANSCRIPT,
  addPage,
  latestWindow,
  windowEnd,
  withWholeBlock,
  type HeldTranscript,
} from '@demicodes/web-ui/transport/protocol'
import { shownWindow } from '@demicodes/web-ui/agent/history'
import { ApiError, apiRequest, readResponse } from '../api/client'
import {
  commandRecordSchema,
  transcriptPageSchema,
  wholeBlockSchema,
  type TranscriptPage,
} from '../api/generated/web-api'
import type { Conversation } from '../state/types'
import { commandTerminal } from './terminals'

/** Where a page is read from (`web-api.md` § Pages). */
type PageAt =
  | { kind: 'latest' }
  | { kind: 'before' | 'after'; index: number; edge: string }
  | { kind: 'around'; block: string }

/**
 * The reads of conversations' history (`web-api.md` § Conversation
 * history), each held where the page shows it
 * (`web-application.md` § Transcript windows). A page that one read is
 * already fetching is not fetched twice; `live` says whether the
 * conversation's stream keeps its root's end, whose length then stands.
 */
export function createHistoryReads(signal: () => AbortSignal, live: (conversation: Conversation) => boolean) {
  const fetching = new Set<string>()

  function path(conversation: Conversation): string {
    return `/conversations/${encodeURIComponent(conversation.id)}/transcript`
  }

  /** One page of the agent `node`'s transcript, the root's for null. */
  async function readPage(conversation: Conversation, node: string | null, at: PageAt): Promise<TranscriptPage> {
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
    const response = await apiRequest(`${path(conversation)}?${query}`, { signal: signal() })
    return readResponse(response, transcriptPageSchema)
  }

  function historyOf(conversation: Conversation, node: string | null): HeldTranscript {
    return node === null
      ? conversation.history
      : conversation.subagents.find((agent) => agent.id === node)?.history ?? EMPTY_TRANSCRIPT
  }

  /** Takes `page` into the agent's history, with what it names beside its blocks. */
  function take(conversation: Conversation, node: string | null, page: TranscriptPage): void {
    const failures = { ...page.failures }
    if (node === null) {
      conversation.history = addPage(conversation.history, page, live(conversation))
      conversation.failures = { ...conversation.failures, ...failures }
      conversation.instructions = page.instructions
      conversation.summaries = {
        ...conversation.summaries,
        ...Object.fromEntries(page.summaries.map((summary) => [summary.marker, summary.summaryTokens])),
      }
      return
    }
    const agent = conversation.subagents.find((candidate) => candidate.id === node)
    if (agent) {
      agent.history = addPage(agent.history, page, agent.phase === 'running')
      agent.failures = { ...agent.failures, ...failures }
    }
  }

  /** Reads a page once at a time, and takes it. */
  async function read(conversation: Conversation, node: string | null, at: PageAt): Promise<void> {
    const key = `${conversation.id}:${node ?? ''}:${JSON.stringify(at)}`
    if (fetching.has(key)) {
      return
    }
    fetching.add(key)
    try {
      take(conversation, node, await readPage(conversation, node, at))
    } finally {
      fetching.delete(key)
    }
  }

  /**
   * Reads the next page at an edge of the window the reader is in
   * (`web-application.md` § Transcript windows). A page the transcript's
   * rewrite refuses drops the windows that no longer join and reads the
   * page around the block in view again (§ Rewrites).
   */
  async function readBeside(conversation: Conversation, node: string | null, side: 'before' | 'after'): Promise<void> {
    const history = historyOf(conversation, node)
    const window = node === null ? shownWindow(history, conversation.shownAt) : shownWindow(history, null)
    const edge = side === 'before' ? window.blocks[0] : window.blocks.at(-1)
    if (!edge || (side === 'before' ? window.start === 0 : windowEnd(window) >= history.length)) {
      return
    }
    const index = side === 'before' ? window.start : windowEnd(window) - 1
    try {
      await read(conversation, node, { kind: side, index, edge: edge.id })
    } catch (error) {
      if (!(error instanceof ApiError) || error.code !== 'transcript_changed') {
        throw error
      }
      await recover(conversation, node, edge.id)
    }
  }

  async function recover(conversation: Conversation, node: string | null, anchor: string): Promise<void> {
    const keep = (history: HeldTranscript): HeldTranscript => {
      const latest = latestWindow(history)
      return { length: history.length, windows: latest ? [latest] : [] }
    }
    if (node === null) {
      conversation.history = keep(conversation.history)
    } else {
      const agent = conversation.subagents.find((candidate) => candidate.id === node)
      if (agent) {
        agent.history = keep(agent.history)
      }
    }
    await readAround(conversation, node, anchor).catch((error: unknown) => {
      // The rewrite removed the block: the latest is what the reader sees now.
      if (!(error instanceof ApiError) || error.code !== 'block_not_found') {
        throw error
      }
      if (node === null) {
        conversation.shownAt = null
      }
    })
  }

  /** Shows the window around the block `block`, reading it first when the page does not hold it. */
  async function readAround(conversation: Conversation, node: string | null, block: string): Promise<void> {
    const held = historyOf(conversation, node).windows.some((window) => window.blocks.some((candidate) => candidate.id === block))
    if (!held) {
      await read(conversation, node, { kind: 'around', block })
    }
    if (node === null) {
      conversation.shownAt = block
    }
  }

  return {
    /** The latest page of the agent `node`'s transcript. */
    readLatest: (conversation: Conversation, node: string | null) => read(conversation, node, { kind: 'latest' }),
    /** The page before the window the reader is in. */
    readBefore: (conversation: Conversation, node: string | null) => readBeside(conversation, node, 'before'),
    /** The page after the window the reader is in. */
    readAfter: (conversation: Conversation, node: string | null) => readBeside(conversation, node, 'after'),
    readAround,
    /** Reads a block whole, for a row the page holds light (`web-api.md` § Light form). */
    async readWhole(conversation: Conversation, node: string | null, id: string): Promise<void> {
      const query = node === null ? '' : `?node=${encodeURIComponent(node)}`
      const response = await apiRequest(`${path(conversation)}/blocks/${encodeURIComponent(id)}${query}`, { signal: signal() })
      const { block, failures } = await readResponse(response, wholeBlockSchema)
      if (node === null) {
        conversation.history = withWholeBlock(conversation.history, block)
        conversation.failures = { ...conversation.failures, ...failures }
        return
      }
      const agent = conversation.subagents.find((candidate) => candidate.id === node)
      if (agent) {
        agent.history = withWholeBlock(agent.history, block)
        agent.failures = { ...agent.failures, ...failures }
      }
    },
    /** Reads an ended command's record, so its terminal tab can open (`web-api.md` § Subagents and commands). */
    async readCommand(conversation: Conversation, commandId: string): Promise<void> {
      const response = await apiRequest(
        `/conversations/${encodeURIComponent(conversation.id)}/commands/${encodeURIComponent(commandId)}`,
        { signal: signal() },
      )
      const record = await readResponse(response, commandRecordSchema)
      if (!conversation.terminals.some((terminal) => terminal.id === record.commandId)) {
        conversation.terminals.push(commandTerminal(record))
      }
    },
  }
}

export type HistoryReads = ReturnType<typeof createHistoryReads>
