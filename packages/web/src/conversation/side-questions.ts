import { toRaw } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { ClientContent } from '@demicodes/protocol'
import type { DraftMessage, MessageFiles } from '@demicodes/web-ui/agent/comments/comments'
import type { NewSideQuestion, ReplyAsking, ShownSideQuestion } from '@demicodes/web-ui/agent/comments/asks'
import { ApiError } from '../api/client'
import { t } from './locales/text'
import {
  askSideQuestion,
  deleteSideQuestion,
  foldSideQuestion,
  followUpSideQuestion,
  retrySideQuestion,
  stopSideQuestion,
} from '../api/side-questions'
import type { Conversation, SentAction, SideRecord } from '../state/types'
import { settleSent, sideActivity, sideRecord } from './side-records'

/** What of a conversation its side questions are: its id and those the page holds. */
type SideHolder = Pick<Conversation, 'id' | 'sides'>

/**
 * How the conversation's page writes a side question's messages: their
 * files among the conversation's, and each message's content once its files
 * are uploaded, which rejects when one cannot be.
 */
export interface SideWriting {
  files: MessageFiles
  content: (message: DraftMessage) => Promise<ClientContent[]>
}

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
    sending: side.sending,
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
 * lane keeps live while the page subscribes to their nodes and the
 * history routes read by their nodes. A question, a follow-up and a fold
 * show at once; a refused one goes back.
 */
export const useSideQuestions = defineStore('sideQuestions', () => {
  /**
   * The last request each side question's actions sent, so that its next
   * action goes after it: a Stop or a deletion never reaches the backend
   * before the question or follow-up it follows.
   */
  const tails = new Map<string, Promise<void>>()

  function serially(id: string, run: () => Promise<void>): Promise<void> {
    const next = (tails.get(id) ?? Promise.resolve()).then(run)
    tails.set(id, next)
    return next
  }

  function sideOf(conversation: SideHolder, id: string): SideRecord | undefined {
    return conversation.sides.find((side) => side.id === id)
  }

  /**
   * The backend took the action `sent` of the side question `side`: it stays
   * on its way until the lane shows it (`settleSent`).
   */
  function taken(side: SideRecord, sent: SentAction): void {
    if (side.sent !== null && toRaw(side.sent) === sent) {
      side.sent.taken = true
      settleSent(side)
    }
  }

  /**
   * Asks a new side question: its card shows at once, working, its files
   * uploading as a message's do, and goes again if the backend refuses it or
   * a file cannot be uploaded.
   */
  async function ask(conversation: SideHolder, writing: SideWriting, question: NewSideQuestion): Promise<void> {
    const { question: message, ...asked } = question
    const side = sideRecord({ ...asked, question: [], picture: question.picture ?? undefined, title: null, folded: false, askedAt: new Date().toISOString() })
    side.sending = { markdown: message.text, attachments: writing.files.capsules(message.fileIds) }
    const sent: SentAction = { heldWhenSent: side.history, taken: false }
    side.sent = sent
    conversation.sides = [...conversation.sides, side]
    // Its files stay while it is on its way.
    const holder = `ask-${question.id}`
    writing.files.hold(holder, message.fileIds)
    await serially(question.id, async () => {
      try {
        const content = await writing.content(message)
        const record = await askSideQuestion(conversation.id, { ...asked, question: content })
        const held = sideOf(conversation, question.id)
        // The entry that names it may have come first, and says the same or later.
        if (held && !held.listed) {
          held.record = record
        }
        if (held) {
          held.sending = null
          taken(held, sent)
        }
      } catch (error) {
        conversation.sides = conversation.sides.filter((each) => each.id !== question.id)
        reportError(t('failures.askSideQuestion'), error, { site: 'failures.askSideQuestion', conversation: conversation.id, userVisible: true })
      } finally {
        writing.files.hold(holder, null)
      }
    })
  }

  /** Sends a follow-up of the side question `id` once its files are uploaded. */
  function followUp(conversation: SideHolder, writing: SideWriting, id: string, message: DraftMessage): Promise<void> {
    const holder = `follow-up-${crypto.randomUUID()}`
    writing.files.hold(holder, message.fileIds)
    return sending(conversation, id, 'failures.sendFollowUp', t('failures.sendFollowUp'), async () => {
      try {
        await followUpSideQuestion(conversation.id, id, await writing.content(message))
      } finally {
        writing.files.hold(holder, null)
      }
    })
  }

  /**
   * Sends the side question `id` the action `send` names, a follow-up or a
   * Retry, while its card shows it working.
   */
  async function sending(conversation: SideHolder, id: string, site: string, title: string, send: () => Promise<void>): Promise<void> {
    const side = sideOf(conversation, id)
    if (!side) {
      return
    }
    const sent: SentAction = { heldWhenSent: side.history, taken: false }
    side.sent = sent
    await serially(id, async () => {
      try {
        await send()
        taken(side, sent)
      } catch (error) {
        if (side.sent !== null && toRaw(side.sent) === sent) {
          side.sent = null
        }
        reportError(title, error, { site, conversation: conversation.id, userVisible: true })
      }
    })
  }

  /**
   * The card's header folds the card and opens it again: at once, and the
   * writes go one at a time, so the backend ends with the user's latest
   * choice, and neither an earlier write's answer nor a read of the list
   * changes what the card shows meanwhile. A refused last write goes back to
   * what the backend last confirmed.
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
        reportError(t('failures.foldSideQuestion'), error, { site: 'failures.foldSideQuestion', conversation: conversation.id, userVisible: true })
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
   * Stops the side question `id`: the backend ends its turn and commands once
   * the question or follow-up on its way has reached it, and the transcript
   * shows the stop.
   */
  async function stop(conversation: SideHolder, id: string): Promise<void> {
    await serially(id, async () => {
      try {
        await stopSideQuestion(conversation.id, id)
      } catch (error) {
        reportError(t('failures.stopSideQuestion'), error, { site: 'failures.stopSideQuestion', conversation: conversation.id, userVisible: true })
      }
    })
  }

  /**
   * Delete on a question still on its first answer: its card is gone at
   * once, and the backend stops it and deletes it; a refused deletion brings
   * the card back.
   */
  async function remove(conversation: SideHolder, id: string): Promise<void> {
    const side = sideOf(conversation, id)
    if (!side) {
      return
    }
    side.deleting = true
    await serially(id, async () => {
      // Its question was refused: no side question was made to delete.
      if (!sideOf(conversation, id)) {
        return
      }
      try {
        await deleteSideQuestion(conversation.id, id)
      } catch (error) {
        // Another page deleted it first: the card is gone either way.
        if (!(error instanceof ApiError && error.status === 404)) {
          side.deleting = false
          reportError(t('failures.deleteSideQuestion'), error, { site: 'failures.deleteSideQuestion', conversation: conversation.id, userVisible: true })
          return
        }
      }
      conversation.sides = conversation.sides.filter((each) => each.id !== id)
    })
  }

  /**
   * What the conversation's replies do with its side questions, `readPage`
   * reading a page of one's transcript by its node.
   */
  function asking(conversation: SideHolder, writing: SideWriting, readPage: (id: string, at: 'latest' | 'before') => void): ReplyAsking {
    return {
      get questions() {
        return conversation.sides.filter((side) => !side.deleting).map(shown)
      },
      ask: (question) => void ask(conversation, writing, question),
      followUp: (id, message) => void followUp(conversation, writing, id, message),
      retry: (id) => void sending(conversation, id, 'failures.retrySideQuestion', t('failures.retrySideQuestion'), () => retrySideQuestion(conversation.id, id)),
      stop: (id) => void stop(conversation, id),
      remove: (id) => void remove(conversation, id),
      fold: (id, folded) => void fold(conversation, id, folded),
      read: readPage,
    }
  }

  return { asking }
})
