import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { NewSideQuestion, ReplyAsking, ShownSideQuestion } from '@demicodes/web-ui/agent/comments/asks'
import { t } from './locales/text'
import {
  askSideQuestion,
  foldSideQuestion,
  followUpSideQuestion,
  retrySideQuestion,
} from '../api/side-questions'
import type { Conversation, SideRecord } from '../state/types'
import { settleSent, sideActivity, sideRecord } from './side-records'
import { pressed, refused } from '@demicodes/web-ui/agent/wait-clock'

/** What of a conversation its side questions are: its id and those the page holds. */
type SideHolder = Pick<Conversation, 'id' | 'sides'>

/** The side question as the transcript shows it. */
function shown(side: SideRecord): ShownSideQuestion {
  const { record } = side
  return {
    id: side.id,
    blockId: record.blockId,
    start: record.start,
    end: record.end,
    quote: record.quote,
    picture: record.picture ?? null,
    question: record.question,
    title: record.title,
    folded: record.folded,
    askedAt: record.askedAt,
    history: side.history,
    failures: side.failures,
    working: side.sent !== null || side.sessionPhase !== 'idle',
    activity: sideActivity(side),
  }
}

/**
 * The conversations' side questions, each a node of the conversation's
 * tree: the conversation's agents name them and keep them current
 * (`./activity.ts`), and their answers are their transcripts, which the
 * socket keeps live while the page subscribes to their nodes and the
 * history routes read by their nodes. A question, a follow-up and a fold
 * show at once; a refused one goes back.
 */
export const useSideQuestions = defineStore('sideQuestions', () => {
  function sideOf(conversation: SideHolder, id: string): SideRecord | undefined {
    return conversation.sides.find((side) => side.id === id)
  }

  /**
   * The backend took the action `id` of the side question `side`: it stays
   * on its way until the socket shows it (`settleSent`).
   */
  function taken(side: SideRecord, id: string): void {
    if (side.sent?.id === id) {
      side.sent.heldWhenTaken = side.history
      settleSent(side)
    }
  }

  /**
   * Asks a new side question, its first message under its own id: its card
   * shows at once, working, and goes again if the backend refuses it.
   */
  async function ask(conversation: SideHolder, question: NewSideQuestion): Promise<void> {
    const side = sideRecord({ ...question, picture: question.picture ?? undefined, title: null, folded: false, askedAt: new Date().toISOString() })
    side.sent = { id: question.id, heldWhenTaken: null }
    pressed(side.waitClock, question.id)
    conversation.sides = [...conversation.sides, side]
    try {
      const record = await askSideQuestion(conversation.id, question)
      const held = sideOf(conversation, question.id)
      // The entry that names it may have come first, and says the same or later.
      if (held && !held.listed) {
        held.record = record
      }
      if (held) {
        taken(held, question.id)
      }
    } catch (error) {
      conversation.sides = conversation.sides.filter((each) => each.id !== question.id)
      reportError(t('failures.askSideQuestion'), error, { userVisible: true })
    }
  }

  /**
   * Sends the side question `id` the action `send` names under an id of the
   * page's, a follow-up or a Retry, while its card shows it working.
   */
  async function sending(conversation: SideHolder, id: string, title: string, send: (action: string) => Promise<void>): Promise<void> {
    const side = sideOf(conversation, id)
    if (!side) {
      return
    }
    const action = crypto.randomUUID()
    side.sent = { id: action, heldWhenTaken: null }
    pressed(side.waitClock, action)
    try {
      await send(action)
      taken(side, action)
    } catch (error) {
      if (side.sent?.id === action) {
        side.sent = null
      }
      refused(side.waitClock, action)
      reportError(title, error, { userVisible: true })
    }
  }

  /**
   * × folds the card into the margin's mark, the mark opens it again: at
   * once, and the writes go one at a time, so the backend ends with the
   * user's latest choice, and neither an earlier write's answer nor a read
   * of the list changes what the card shows meanwhile. A refused last write
   * goes back to what the backend last confirmed.
   */
  async function fold(conversation: SideHolder, id: string, folded: boolean): Promise<void> {
    const side = sideOf(conversation, id)
    if (!side) {
      return
    }
    side.folding ??= { intent: folded, confirmed: side.record.folded, writes: 0, chain: Promise.resolve() }
    const folding = side.folding
    folding.intent = folded
    folding.writes += 1
    side.record = { ...side.record, folded }
    const write = folding.chain.then(async () => {
      try {
        const record = await foldSideQuestion(conversation.id, id, folded)
        folding.confirmed = record.folded
        side.record = { ...record, folded: folding.intent }
      } catch (error) {
        if (folding.writes === 1) {
          folding.intent = folding.confirmed
          side.record = { ...side.record, folded: folding.confirmed }
        }
        reportError(t('failures.foldSideQuestion'), error, { userVisible: true })
      } finally {
        folding.writes -= 1
        if (folding.writes === 0) {
          side.folding = null
        }
      }
    })
    folding.chain = write
    await write
  }

  /**
   * What the conversation's replies do with its side questions, `readPage`
   * reading a page of one's transcript by its node.
   */
  function asking(conversation: SideHolder, readPage: (id: string, at: 'latest' | 'before') => void): ReplyAsking {
    return {
      get questions() {
        return conversation.sides.map(shown)
      },
      ask: (question) => void ask(conversation, question),
      followUp: (id, text) => void sending(conversation, id, t('failures.sendFollowUp'), (action) => followUpSideQuestion(conversation.id, id, action, text)),
      retry: (id) => void sending(conversation, id, t('failures.retrySideQuestion'), (action) => retrySideQuestion(conversation.id, id, action)),
      fold: (id, folded) => void fold(conversation, id, folded),
      read: readPage,
    }
  }

  return { asking }
})
