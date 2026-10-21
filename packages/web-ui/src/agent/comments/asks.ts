import type { Block, MediaSource, ProviderFailureFacts } from '@demicodes/protocol'
import type { HeldTranscript } from '../../transport/protocol'
import { toolCallTitle } from '../block-helpers'
import { plainUserMarkdown } from '../../markdown/user-markdown'
import { latestWindow } from '@demicodes/conversation-client'
import type { HeldBlock } from '../history'
import type { AgentActivity } from '../agent-activity'
import { t } from '../locales/text'

/**
 * Side questions on a reply's words. Each is a side conversation from the
 * conversation's history through the asked reply. Its card shows each question
 * and its final answer under the block holding the words, and folds into a mark
 * in the margin.
 */

/** A side question as the transcript shows it. */
export interface ShownSideQuestion {
  id: string
  /** The reply's text block the words are in. */
  blockId: string
  start: number
  end: number
  quote: string
  /**
   * The picture a side question about a picture asks of, whose place among
   * the block's pictures `start` and `end` are; absent for words.
   */
  picture?: MediaSource | null
  /** The first question; a follow-up is a `user` block of its transcript. */
  question: string
  /** The small model's title; null until it has one, when the card says Ask. */
  title: string | null
  folded: boolean
  /**
   * When it was asked: its own blocks start then, after the history it started
   * from.
   */
  askedAt: string
  /**
   * Its transcript as the page holds it, by pages: the history it started from,
   * then its own turns.
   */
  history: HeldTranscript
  /** What the backend knows of its failures, by their blocks. */
  failures: Readonly<Record<string, ProviderFailureFacts>>
  /** A turn of it runs, or the page's question or follow-up is on its way. */
  working: boolean
  /** What its agent does now, which its window shows after its transcript. */
  activity: AgentActivity
}

/** A new side question, under the id the page chose. */
export interface NewSideQuestion {
  id: string
  blockId: string
  start: number
  end: number
  quote: string
  picture?: MediaSource | null
  question: string
}

/**
 * What the transcript's host does with its side questions; absent where a
 * transcript takes none, as a share's page or a subagent's.
 */
export interface ReplyAsking {
  /**
   * The side questions whose block the transcript's current path holds, in the
   * order they were asked.
   */
  questions: readonly ShownSideQuestion[]
  ask: (question: NewSideQuestion) => void
  followUp: (id: string, text: string) => void
  retry: (id: string) => void
  /**
   * × folds the card into the margin's mark; the mark or the words open it
   * again.
   */
  fold: (id: string, folded: boolean) => void
  /**
   * Reads a page of its transcript: its latest for a card the page holds
   * none of, or the one before the window its full conversation shows as
   * the reader nears its start.
   */
  read: (id: string, at: 'latest' | 'before') => void
  /**
   * ↗: the whole side conversation, its steps included, in the window a
   * subagent's opens in, which the session view keeps. A list shown on its own
   * leaves it out and ↗ does nothing.
   */
  openFull?: (id: string) => void
}

/** One question and its answer as the card shows them. */
export interface CardTurn {
  question: string
  /**
   * The one status line while the answer is being made; null once it answers.
   */
  status: string | null
  answer: string
  streaming: boolean
}

type ErrorBlock = Extract<Block, { type: 'error' }>

/** What a card shows: each turn, and the failure the last one ended with. */
export interface CardContent {
  turns: CardTurn[]
  failure: ErrorBlock | null
}

function userText(block: Extract<Block, { type: 'user' }>): string {
  return block.content.flatMap((part) => (part.type === 'text' ? [part.text] : [])).join('\n')
}

/**
 * The side question's own blocks the page holds, from its first message on:
 * the user block whose turn is the side question's id, as the backend names
 * it, whatever pages the page holds. Null while the window the page holds
 * does not reach back to it.
 */
export function ownBlocks(question: ShownSideQuestion): readonly HeldBlock[] | null {
  const blocks = latestWindow(question.history)?.blocks ?? []
  const first = blocks.findIndex((block) => block.type === 'user' && block.turnId === question.id)
  return first < 0 ? null : blocks.slice(first)
}

/**
 * The page to read for the card of `question`:
 *
 * - the latest, when the page holds none of its transcript
 * - the one before, when what it holds starts after the first question
 * - none, once it holds the first question, or while the question is on its way
 *   and its transcript arrives on the socket
 */
export function pageToRead(question: ShownSideQuestion): 'latest' | 'before' | null {
  const window = latestWindow(question.history)
  if (!window)
    return question.working ? null : 'latest'
  return ownBlocks(question) === null && window.start > 0 ? 'before' : null
}

/**
 * The read `pageToRead` asks for, named with where the held window starts,
 * so each read is asked once and the next one follows when a page joined.
 */
export function readKey(question: ShownSideQuestion): string | null {
  const page = pageToRead(question)
  return page && `${question.id}:${page}:${latestWindow(question.history)?.start ?? ''}`
}

/**
 * The card of `question` from its transcript: each of its own messages, the
 * first shown as the question the user typed, with the last text its turn wrote
 * as the answer. While the last turn works, before its answer starts, the one
 * status line names what it does, the call it runs or that it thinks. Before
 * the page holds its first message, the question alone.
 */
export function cardContent(question: ShownSideQuestion): CardContent {
  const own = (ownBlocks(question) ?? []).filter((block) => block.type !== 'response')
  const turns: { question: string; blocks: Block[] }[] = []
  for (const block of own) {
    if (block.type === 'user')
      turns.push({ question: turns.length === 0 ? question.question : userText(block), blocks: [] })
    else
      turns.at(-1)?.blocks.push(block)
  }
  if (turns.length === 0)
    turns.push({ question: question.question, blocks: [] })
  const last = turns.at(-1)!
  const ended = last.blocks.at(-1)
  const failure = !question.working && ended?.type === 'error' ? ended : null
  return {
    turns: turns.map((turn) => {
      const texts = turn.blocks.filter((block) => block.type === 'text')
      const answer = texts.at(-1)?.text ?? ''
      const running = question.working && turn === last
      const writing = running && turn.blocks.at(-1)?.type === 'text'
      if (!running || writing)
        return { question: turn.question, status: null, answer, streaming: writing }
      const step = turn.blocks.at(-1)
      const status = step?.type === 'tool_call' ? toolCallTitle(step) : t('ask.thinking')
      return { question: turn.question, status, answer: '', streaming: false }
    }),
    failure,
  }
}

/**
 * The side question's conversation as its full window shows it: its own turns
 * only.
 *
 * - The first message is drawn as the user's question with its quote, the asked
 *   picture for one about a picture.
 * - Neither the instruction the model reads nor the history before it, which is
 *   the conversation's own, is shown.
 * - The questions were typed in a plain field and show as typed, as a comment's
 *   words do, never read as Markdown.
 */
export function shownTurns(question: ShownSideQuestion): HeldBlock[] {
  // The first shows as a comment does: the quoted words or a small copy of the
  // picture, the question under it.
  const first = (block: Extract<HeldBlock, { type: 'user' }>): HeldBlock => ({
    ...block,
    content: [],
    comments: [{
      id: `${block.id}-quote`, blockId: question.blockId, start: question.start, end: question.end, at: 0,
      quote: question.quote, ...(question.picture ? { picture: question.picture } : {}), text: question.question,
    }],
  })
  // A follow-up's text is the field's plain text, which the bubble reads as a
  // message's Markdown.
  const followUp = (block: Extract<HeldBlock, { type: 'user' }>): HeldBlock => ({
    ...block,
    content: block.content.map((part) => (part.type === 'text' ? { ...part, text: plainUserMarkdown(part.text) } : part)),
  })
  return (ownBlocks(question) ?? []).map((block, index) => {
    if (block.type !== 'user')
      return block
    return index === 0 ? first(block) : followUp(block)
  })
}
