import { ref } from 'vue'
import { countOf, upperFirst } from '@demicodes/utils'
import type { SentenceText } from '../ui/ui-text'
import { isSubagentRunning, type SubagentRecord } from '../agent/subagents'
import { isTerminalRunning, type TerminalRecord } from '../agent/terminals'
import type { ConversationState } from '../agent/types'

/** Why a conversation's move controls are unavailable: it is archived, or a move or change of it is under way. */
export type MoveLock = 'archived' | 'moving'

/** What a lock says on the controls it holds (`product.md` § Where a conversation runs). */
export const MOVE_LOCK_REASON: Record<MoveLock, SentenceText> = {
  archived: 'This conversation is archived.',
  moving: 'This conversation is moving.',
}

/** What still runs when the question is asked, as the page shows it: moving stops all of it. */
export interface WorkToStop {
  /** The root agent has a turn under way. */
  agent: boolean
  subagents: number
  commands: number
}

/**
 * What a conversation still runs, as the page shows it; none once its agent,
 * its subagents and its commands have all ended. A command counts as work
 * because its job holds the conversation's file gate until its runner says it
 * ended, and the backend moves no conversation while it is held
 * (`sessions-and-targets.md` § Switch the primary target).
 */
export function workToStop(conversation: {
  phase: ConversationState['phase']
  subagents: readonly Pick<SubagentRecord, 'phase'>[]
  terminals: readonly Pick<TerminalRecord, 'phase'>[]
}): WorkToStop | null {
  const work = {
    agent: conversation.phase !== 'idle',
    subagents: conversation.subagents.filter((agent) => isSubagentRunning(agent.phase)).length,
    commands: conversation.terminals.filter((terminal) => isTerminalRunning(terminal.phase)).length,
  }
  return work.agent || work.subagents > 0 || work.commands > 0 ? work : null
}

/**
 * What stops, as the dialog's first line says it
 * (`product.md` § Where a conversation runs): *The agent, 1 subagent and 2
 * commands will stop.*
 */
export function stopSentence(work: WorkToStop): string {
  const parts = [
    ...(work.agent ? ['the agent'] : []),
    ...(work.subagents > 0 ? [countOf(work.subagents, 'subagent')] : []),
    ...(work.commands > 0 ? [countOf(work.commands, 'command')] : []),
  ]
  const last = parts.pop()
  const list = parts.length ? `${parts.join(', ')} and ${last}` : (last ?? 'the agent')
  return `${upperFirst(list)} will stop.`
}

/** A move the dialog asks about: where the conversation goes, and where it leaves. */
export interface MoveQuestion {
  /** What the move stops first, while the agent works; none for a conversation at rest. */
  stops?: WorkToStop
  /** The Host it moves to, by name. */
  host: string
  /** The directory it will run in there, as a person reads it; none for one of its own the Host names. */
  directory: string | null
  /** The Host it leaves, by name. */
  from: string
  fromCloud: boolean
}

/**
 * The question before a conversation that has messages moves, or whose
 * agent works (`product.md` § Where a conversation runs): `ask` opens
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
