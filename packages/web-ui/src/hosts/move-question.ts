import { pageLanguage } from '../i18n/text'
import { t } from './locales/text'
import { ref } from 'vue'
import { upperFirst } from '@demicodes/utils'
import { operations, type Operation } from '../ui/operations'
import type { SentenceText } from '../ui/ui-text'
import { isSubagentRunning, type SubagentRecord } from '../agent/subagents'
import { isTerminalRunning, type TerminalRecord } from '../agent/terminals'
import type { ConversationState } from '../agent/types'

/** Why a conversation's move controls are unavailable: it is archived, or a move or change of it is under way. */
export type MoveLock = 'archived' | 'moving'

/** What a lock says on the controls it holds (`hosts.md` § Where a conversation runs). */
export const MOVE_LOCK_REASON: Record<MoveLock, SentenceText> = {
  get archived() {
    return t('lock.archived')
  },
  get moving() {
    return t('lock.moving')
  },
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
 * (`sessions-and-targets.md` § Switch the primary target). None either while
 * the backend says the agent only waits for the conversation's offline
 * device, `onlyWaitsForHost`: a move fails the commands that wait, which
 * never ran, and lets the agent go on on the new Host.
 */
export function workToStop(conversation: {
  phase: ConversationState['phase']
  subagents: readonly Pick<SubagentRecord, 'phase'>[]
  terminals: readonly Pick<TerminalRecord, 'phase'>[]
  onlyWaitsForHost: boolean
}): WorkToStop | null {
  if (conversation.onlyWaitsForHost) {
    return null
  }
  const work = {
    agent: conversation.phase !== 'idle',
    subagents: conversation.subagents.filter((agent) => isSubagentRunning(agent.phase)).length,
    commands: conversation.terminals.filter((terminal) => isTerminalRunning(terminal.phase)).length,
  }
  return work.agent || work.subagents > 0 || work.commands > 0 ? work : null
}

/** The locale a list is joined in: English as the Writing page writes it, without the comma before and. */
function listLocale(): string {
  const language = pageLanguage()
  return language === 'en' ? 'en-GB' : language
}

/**
 * What stops, as the dialog's first line says it
 * (`hosts.md` § Where a conversation runs): *The agent, 1 subagent and 2
 * commands will stop.*
 */
export function stopSentence(work: WorkToStop): string {
  const parts = [
    ...(work.agent ? [t('stop.agent')] : []),
    ...(work.subagents > 0 ? [t('stop.subagents', { count: work.subagents }, work.subagents)] : []),
    ...(work.commands > 0 ? [t('stop.commands', { count: work.commands }, work.commands)] : []),
  ]
  const list = parts.length ? new Intl.ListFormat(listLocale(), { type: 'conjunction' }).format(parts) : t('stop.agent')
  return t('stop.sentence', { list: upperFirst(list) })
}

/** The operations of the question's two answers. */
export interface MoveAnswers {
  move: Operation
  tell: Operation
}

/** How many move questions the page has made, which names each one's answers. */
let questions = 0

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
 * agent works (`hosts.md` § Where a conversation runs): `ask` opens
 * `MoveConversationDialog` and answers whether the conversation moved, once
 * the user cancelled or `carryOut` finished the move the user chose, told to
 * the agent or not. The question stays once answered, so the dialog leaves
 * showing it.
 */
export function useMoveQuestion() {
  const question = ref<MoveQuestion | null>(null)
  const open = ref(false)
  /** Move and Move and Tell Agent: one group, so while one works the other cannot be pressed (`operations.ts`). */
  const target = `move:${++questions}`
  const answers: MoveAnswers = {
    move: { action: 'move', target, group: 'answer' },
    tell: { action: 'tell', target, group: 'answer' },
  }
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

  /** Carries out the answer; once it is done, the dialog closes if it still asks this question. */
  async function answer(tell: boolean): Promise<void> {
    const asked = pending
    if (!asked) {
      return
    }
    await operations.run(tell ? answers.tell : answers.move, async () => {
      try {
        asked.answer(await asked.carryOut(tell))
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
