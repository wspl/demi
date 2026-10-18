import { watch } from 'vue'
import { defineStore } from 'pinia'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { NewSideQuestion, ReplyAsking, ShownSideQuestion } from '@demicodes/web-ui/agent/comments/asks'
import { t } from './locales/text'
import {
  askSideQuestion,
  foldSideQuestion,
  followUpSideQuestion,
  readSideQuestions,
  retrySideQuestion,
} from '../api/side-questions'
import type { SideQuestion, SideQuestions } from '../api/generated/web-api'
import { useProduct } from '../state/product'
import { isNewer } from '../state/revisions'
import type { Conversation, SideRecord } from '../state/types'
import { sideRecord } from './side-records'

/** What of a conversation its side questions are: its id, the list the page holds and its revision. */
type SideHolder = Pick<Conversation, 'id' | 'sides' | 'sidesHeld'>

/** The side question as the transcript shows it. */
function shown(side: SideRecord & { record: SideQuestion }): ShownSideQuestion {
  const { record } = side
  return {
    id: side.id,
    blockId: record.blockId,
    start: record.start,
    end: record.end,
    quote: record.quote,
    question: record.question,
    title: record.title,
    folded: record.folded,
    askedAt: record.askedAt,
    history: side.history,
    failures: side.failures,
    working: side.sending || side.sessionPhase !== 'idle',
  }
}

/**
 * The conversations' side questions (`comments-and-ask.md` § Side
 * questions, `web-api-conversations.md` § Side questions): a page reads
 * them for a conversation it shows, and again whenever the summary's
 * `sideQuestionsRevision` is newer than the revision it holds
 * (`web-api-sync.md` § Revisions counted in memory); an answer older than
 * the one it holds is dropped. Their answers are their transcripts, which
 * the socket keeps live and the history routes read by their nodes. A
 * question, a follow-up and a fold show at once; a refused one goes back.
 */
export const useSideQuestions = defineStore('sideQuestions', () => {
  const product = useProduct()
  /** The conversations a page shows, whose side questions it follows, by id. */
  const followed = new Map<string, SideHolder>()
  /** The reads on their way, each with whether a newer revision was heard of meanwhile. */
  const reading = new Map<string, { again: boolean }>()

  /** Takes an answer newer than the one the page holds, keeping what it holds of each transcript. */
  function take(conversation: SideHolder, answer: SideQuestions): void {
    const taken = { run: product.snapshot?.run ?? null, revision: answer.revision }
    if (!isNewer(taken, conversation.sidesHeld)) {
      return
    }
    conversation.sidesHeld = taken
    const held = new Map(conversation.sides.map((side) => [side.id, side]))
    const listed = answer.sideQuestions.map((record) => {
      const side = held.get(record.id)
      if (!side) {
        return sideRecord(record)
      }
      // A fold on its way stays as the user chose it.
      side.record = side.folding ? { ...record, folded: side.folding.intent } : record
      return side
    })
    // A question this page sent that the answer was read before, and one
    // whose frames came before the list named it, stay until a later answer
    // names them.
    const unlisted = conversation.sides.filter((side) => (side.sending || side.record === null) && !listed.some((each) => each.id === side.id))
    conversation.sides = [...listed, ...unlisted]
  }

  function behind(conversation: SideHolder): boolean {
    const summary = product.snapshot?.conversations.find((item) => item.id === conversation.id)
    const run = product.snapshot?.run ?? null
    return summary !== undefined && isNewer({ run, revision: summary.sideQuestionsRevision }, conversation.sidesHeld)
  }

  /** Reads the list, unless a read is on its way: that one reads again after, if the summary is still newer. */
  async function read(conversation: SideHolder): Promise<void> {
    const running = reading.get(conversation.id)
    if (running) {
      running.again = true
      return
    }
    const current = { again: false }
    reading.set(conversation.id, current)
    try {
      take(conversation, await readSideQuestions(conversation.id))
    } catch (error) {
      reportError(t('failures.readSideQuestions'), error)
    } finally {
      reading.delete(conversation.id)
    }
    if (current.again && behind(conversation)) {
      await read(conversation)
    }
  }

  /** Reads the conversation's side questions once a page shows it; the summary keeps them current from then on. */
  function follow(conversation: SideHolder): void {
    if (!followed.has(conversation.id)) {
      followed.set(conversation.id, conversation)
      void read(conversation)
    }
  }

  watch(
    () => product.snapshot?.conversations,
    (summaries) => {
      for (const summary of summaries ?? []) {
        const conversation = followed.get(summary.id)
        if (conversation && behind(conversation)) {
          void read(conversation)
        }
      }
    },
  )

  function sideOf(conversation: SideHolder, id: string): SideRecord | undefined {
    return conversation.sides.find((side) => side.id === id)
  }

  /** Asks a new side question: its card shows at once, working, and goes again if the backend refuses it. */
  async function ask(conversation: SideHolder, question: NewSideQuestion): Promise<void> {
    const side = sideRecord({ ...question, title: null, folded: false, askedAt: new Date().toISOString() })
    side.sending = true
    conversation.sides = [...conversation.sides, side]
    try {
      const record = await askSideQuestion(conversation.id, question)
        const held = sideOf(conversation, question.id)
      if (held) {
        held.record = record
      }
    } catch (error) {
      conversation.sides = conversation.sides.filter((each) => each.id !== question.id)
      reportError(t('failures.askSideQuestion'), error, { userVisible: true })
    } finally {
      const held = sideOf(conversation, question.id)
      if (held) {
        held.sending = false
      }
    }
  }

  /** Runs `send` for the side question `id` while its card shows it working. */
  async function sending(conversation: SideHolder, id: string, title: string, send: () => Promise<void>): Promise<void> {
    const side = sideOf(conversation, id)
    if (!side) {
      return
    }
    side.sending = true
    try {
      await send()
    } catch (error) {
      reportError(title, error, { userVisible: true })
    } finally {
      side.sending = false
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
    if (!side?.record) {
      return
    }
    side.folding ??= { intent: folded, confirmed: side.record.folded, writes: 0, chain: Promise.resolve() }
    const folding = side.folding
    folding.intent = folded
    folding.writes += 1
    side.record = { ...side.record, folded }
    const held = side as SideRecord & { record: SideQuestion }
    const write = folding.chain.then(async () => {
      try {
        const record = await foldSideQuestion(conversation.id, id, folded)
        folding.confirmed = record.folded
        side.record = { ...record, folded: folding.intent }
      } catch (error) {
        if (folding.writes === 1) {
          folding.intent = folding.confirmed
          held.record = { ...held.record, folded: folding.confirmed }
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
        return conversation.sides.flatMap((side) => (side.record ? [shown({ ...side, record: side.record })] : []))
      },
      ask: (question) => void ask(conversation, question),
      followUp: (id, text) => void sending(conversation, id, t('failures.sendFollowUp'), () => followUpSideQuestion(conversation.id, id, text)),
      retry: (id) => void sending(conversation, id, t('failures.retrySideQuestion'), () => retrySideQuestion(conversation.id, id)),
      fold: (id, folded) => void fold(conversation, id, folded),
      read: readPage,
    }
  }

  return { follow, asking }
})
