import { t } from './locales/text'
import { ref } from 'vue'
import { operations, type Operation } from '../ui/operations'
import type { SentenceText } from '../ui/ui-text'

/** Why a conversation's move controls are unavailable: it is archived, or a move or change of it is under way. */
export type MoveLock = 'archived' | 'moving'

/** What a lock says on the controls it holds. */
export const MOVE_LOCK_REASON: Record<MoveLock, SentenceText> = {
  get archived() {
    return t('lock.archived')
  },
  get moving() {
    return t('lock.moving')
  },
}

/** The operation of the question's answer, Move. */
export interface MoveAnswers {
  move: Operation
}

/** How many move questions the page has made, which names each one's answer. */
let questions = 0

/** A move the dialog asks about: where the conversation goes, and where it leaves. */
export interface MoveQuestion {
  /** The Host it moves to, by name. */
  host: string
  /** The directory it will run in there, as a person reads it; none for one of its own the Host names. */
  directory: string | null
  /** The Host it leaves, by name. */
  from: string
  fromCloud: boolean
}

/**
 * The question before a conversation that has messages moves, whether or not its
 * agent works: the move stops nothing (`docs/decisions/0027-a-move-applies-at-once.md`).
 *
 * `ask` opens `MoveConversationDialog` and answers whether the conversation moved,
 * once the user cancelled or `carryOut` finished the move. The question stays once
 * answered, so the dialog leaves showing it.
 */
export function useMoveQuestion() {
  const question = ref<MoveQuestion | null>(null)
  const open = ref(false)
  /** Move, which cannot be pressed again while it works (`operations.ts`). */
  const answers: MoveAnswers = {
    move: { action: 'move', target: `move:${++questions}` },
  }
  let pending: {
    carryOut: () => Promise<boolean>
    answer: (moved: boolean) => void
  } | null = null

  function ask(asked: MoveQuestion, carryOut: () => Promise<boolean>): Promise<boolean> {
    // A question still open is cancelled by the newer one.
    pending?.answer(false)
    question.value = asked
    open.value = true
    return new Promise((answer) => {
      pending = { carryOut, answer }
    })
  }

  /** Carries out the move; once it is done, the dialog closes if it still asks this question. */
  async function answer(): Promise<void> {
    const asked = pending
    if (!asked) {
      return
    }
    await operations.run(answers.move, async () => {
      try {
        asked.answer(await asked.carryOut())
      } finally {
        if (pending === asked) {
          open.value = false
          pending = null
        }
      }
    })
  }

  /** Cancel, Escape or the scrim: a move under way goes on and answers when it is done, as a sent mail goes on sending. */
  function cancel(): void {
    open.value = false
    if (operations.blocked(answers.move)) {
      return
    }
    pending?.answer(false)
    pending = null
  }

  return { question, open, answers, ask, answer, cancel }
}
