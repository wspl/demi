import { ref } from 'vue'
import type { SentenceText } from '../ui/ui-text'

/** Why a conversation cannot move while its work runs, as its move controls say. */
export const MOVE_LOCKED: SentenceText = 'This conversation can move once its work ends.'

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
 * The question before a conversation that has messages moves
 * (`product.md` § Where a conversation runs): `ask` opens
 * `MoveConversationDialog` and answers whether the conversation moved, once
 * the user cancelled or `carryOut` finished the move the user chose, told to
 * the agent or not. The question stays once answered, so the dialog leaves
 * showing it.
 */
export function useMoveQuestion() {
  const question = ref<MoveQuestion | null>(null)
  const open = ref(false)
  const busy = ref<'move' | 'tell' | null>(null)
  let pending: {
    carryOut: (tell: boolean) => Promise<boolean>
    answer: (moved: boolean) => void
  } | null = null

  function ask(asked: MoveQuestion, carryOut: (tell: boolean) => Promise<boolean>): Promise<boolean> {
    // A question still open is cancelled by the newer one.
    pending?.answer(false)
    question.value = asked
    open.value = true
    return new Promise((answer) => {
      pending = { carryOut, answer }
    })
  }

  async function answer(tell: boolean): Promise<void> {
    const asked = pending
    if (!asked || busy.value) {
      return
    }
    busy.value = tell ? 'tell' : 'move'
    try {
      asked.answer(await asked.carryOut(tell))
    } finally {
      busy.value = null
      open.value = false
      pending = null
    }
  }

  function cancel(): void {
    if (busy.value) {
      return
    }
    pending?.answer(false)
    pending = null
    open.value = false
  }

  return { question, open, busy, ask, answer, cancel }
}
