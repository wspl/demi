import type { SentenceText } from '../ui/ui-text'
import type { ShownError } from '../infra/errors'
import { isCompactionDivider } from './visible-blocks'
import type { HostMenuHost } from '../hosts/types'

/** Sidebar conversation list, or a session that is not reconnecting. */
export type ListLoad = 'ready' | 'loading' | 'failed'

/**
 * How far the conversation's opening has come (`web-application.md` § Opening
 * a conversation shows in two steps), and the state of its connection since:
 * - `loading`: its history or its agents are not read yet; the pane says it
 *   loads.
 * - `opening`: the first step. The transcript and everything that names its
 *   agents show; the session is opening, and nothing only the live session
 *   knows shows yet.
 * - `connecting`: the first step, whose session could not be reached yet and
 *   is tried again; the transcript's tail says it connects.
 * - `ready`: the second step, the session open and shown; or a conversation
 *   that opens no session, which stops at the first step.
 * - `reconnecting`: an open session whose connection was lost; what it showed
 *   stays while it is tried again.
 * - `failed`: the opening failed, or the session refused to open.
 */
export type SessionLoad = ListLoad | 'opening' | 'connecting' | 'reconnecting'

/**
 * Whether the session has told the conversation's turn, which the Resume
 * chip reads: not while a first opening has yet to open it, nor once it
 * failed; a reconnection keeps what the session told.
 */
export function sessionShown(load: SessionLoad): boolean {
  return load === 'ready' || load === 'reconnecting'
}

/**
 * Whether the conversation shows its transcript: from the first step of its
 * opening on, with its agents, the composer, its marking read and a reveal
 * of a message in it (`web-application.md` § Opening a conversation shows in
 * two steps), unless its opening failed.
 */
export function transcriptShown(load: SessionLoad): boolean {
  return load !== 'loading' && load !== 'failed'
}

/** Whether the conversation is still opening for the first time: what joins it now is not new. */
export function isOpening(load: SessionLoad): boolean {
  return load === 'loading' || load === 'opening' || load === 'connecting'
}

/**
 * What the session pane shows instead of the transcript. `empty` is an open
 * conversation with no messages yet: the composer under it is the way in.
 * `none` is the chat route with no conversation open at all, so the pane
 * itself offers to start one.
 */
export type SessionStatusKind = 'loading' | 'failed' | 'empty' | 'missing' | 'none'

/**
 * The pane that replaces the transcript. `loading` always wins so a restore
 * never reads as an empty conversation, even with cached blocks. Connecting and reconnecting
 * keep the transcript (or an empty list) and say so in the tail row. A failure with
 * history in memory also keeps the transcript: the dock's notice names the
 * failure and offers Retry, and nothing the reader already had disappears.
 */
export function sessionPaneStatus(
  load: SessionLoad,
  hasTranscript: boolean,
): SessionStatusKind | null {
  if (load === 'loading') {
    return 'loading'
  }
  if (load === 'failed') {
    return hasTranscript ? null : 'failed'
  }
  if (load === 'reconnecting' || load === 'connecting') {
    return null
  }
  return hasTranscript ? null : 'empty'
}

/** A session-level failure told at the tail of the transcript. */
export interface SessionFailureNotice {
  label: SentenceText
  /** Retry reopens the session; a refused request has nothing to reopen. */
  retry: boolean
  /** The request the failure came from, such as the socket that refused a frame. */
  requestId: string | null
}

/**
 * What the transcript's tail notice says for a session-level failure, or null
 * when another surface already says it: the status pane (no transcript to
 * keep) or the error record at the tail (it already says it).
 */
export function sessionFailureNotice(
  load: SessionLoad,
  lastError: ShownError | null,
  hasTranscript: boolean,
  tailIsErrorRecord: boolean,
): SessionFailureNotice | null {
  if (!lastError) {
    return null
  }
  if (load === 'failed') {
    return hasTranscript ? { label: lastError.message, retry: true, requestId: lastError.requestId } : null
  }
  if (load === 'loading' || tailIsErrorRecord) {
    return null
  }
  return { label: lastError.message, retry: false, requestId: lastError.requestId }
}

/** The chat route when the sidebar list is not ready, or the id is unknown. */
export function conversationPageKind(
  list: ListLoad,
  found: boolean,
): SessionStatusKind | 'session' {
  if (found) {
    return 'session'
  }
  if (list === 'loading') {
    return 'loading'
  }
  if (list === 'failed') {
    return 'failed'
  }
  return 'missing'
}

/**
 * Why a conversation did not open when the server could not read its
 * database (`conversation_unreadable`), under “Couldn’t load this
 * conversation.”: the page's words, never the server's, which name its files
 * and an operator's command.
 */
export const CONVERSATION_UNREADABLE: SentenceText = 'The server could not read this conversation’s history.'

export function sessionStatusCopy(kind: SessionStatusKind): {
  label: SentenceText
  action?: 'retry' | 'create'
} {
  if (kind === 'loading') {
    return { label: 'Loading conversation…' }
  }
  if (kind === 'failed') {
    return {
      label: 'Couldn’t load this conversation.',
      action: 'retry',
    }
  }
  if (kind === 'missing') {
    return {
      label: 'Conversation not found.',
      action: 'create',
    }
  }
  if (kind === 'none') {
    return {
      label: 'No conversation open.',
      action: 'create',
    }
  }
  return { label: 'No messages yet.' }
}

/**
 * The recovery the dock offers for a turn that did not finish (`product.md`
 * § Recovering an unfinished turn), from the record that ended the last
 * turn among the visible `blocks`: an error resumes, the user's own Stop
 * continues, and anything else is finished or still running. An error that
 * ended no turn, such as a Compact whose summary request failed, a
 * compaction's boundary and marker, and the agent messages and command
 * reports a failed turn took after it ended leave the turn before them as it
 * was (`failures-and-recovery.md` § The unfinished turn). Both are the
 * session's `resume`; the word tells the cause.
 */
export function turnRecovery(
  phase: 'idle' | string,
  blocks: readonly RecordCandidate[],
): 'resume' | 'continue' | null {
  if (phase !== 'idle') {
    return null
  }
  const end = turnRecord(blocks)
  if (end?.type === 'error') {
    return 'resume'
  }
  return end?.type === 'abort' ? 'continue' : null
}

/**
 * Whether the unfinished turn among `blocks` failed because its request was
 * too large for the vendor (`failures-and-recovery.md` § Retries): Compact
 * then summarizes the media away at any share, and Resume continues the turn.
 */
export function refusedForSize(blocks: readonly RecordCandidate[]): boolean {
  const end = turnRecord(blocks)
  return end?.type === 'error' && end.code === 'request_too_large'
}

/**
 * The account a usage limit that ended the unfinished turn offers to go on
 * with, beside Resume, on an entry without rotation (`providers.md`
 * § Account rotation): the one its failure's facts name. It reads the
 * turn's record as Resume does, so input written behind the failure, such
 * as a child's report, keeps it. Null while running, and for any other end.
 */
export function continueWithOffer(
  phase: 'idle' | string,
  blocks: readonly (RecordCandidate & { id?: string; model?: { providerId: string } })[],
  failures: Readonly<Record<string, { continueWith?: { credentialId: string; label: string } | null }>>,
): { providerId: string; credentialId: string; label: string } | null {
  if (phase !== 'idle') {
    return null
  }
  const end = turnRecord(blocks)
  if (end?.type !== 'error' || end.code !== 'usage_limit' || !end.id || !end.model) {
    return null
  }
  const offered = failures[end.id]?.continueWith
  return offered ? { providerId: end.model.providerId, ...offered } : null
}

/** What `turnRecovery` reads of a block. */
interface RecordCandidate {
  type: string
  outsideTurn?: boolean
  code?: string | null
  placement?: string
}

/** The record that ended the last turn among `blocks`, or the block that ended it otherwise. */
function turnRecord<T extends RecordCandidate>(blocks: readonly T[]): T | undefined {
  return blocks.findLast(
    (block) => !isCompactionDivider(block)
      && !(block.type === 'error' && block.outsideTurn)
      && !isLateInput(block),
  )
}

/**
 * Whether the transcript ends with an error record, read past the agent
 * messages and command reports a failed root's turn took after it ended
 * (`failures-and-recovery.md` § The unfinished turn): the record still says
 * the failure, so nothing below repeats it.
 */
export function endsWithErrorRecord(blocks: readonly RecordCandidate[]): boolean {
  return blocks.findLast((block) => !isLateInput(block))?.type === 'error'
}

/** Input a turn takes without opening one: an agent message, or command reports placed as a steer. */
function isLateInput(block: RecordCandidate): boolean {
  return block.type === 'agent_message' || (block.type === 'wakeup' && block.placement === 'steer')
}

/**
 * The Host the dock's Resume waits for (`product.md` § Recovering an
 * unfinished turn): the turn ended unfinished because a device was offline,
 * and the conversation's current primary Host, where the resume runs, is a
 * device that is not online; that is the device the record names unless the
 * conversation moved since. A stopped Cloud wakes for the resume, so it holds
 * nothing back. Null when Resume may go.
 */
export function resumeWaitsFor(
  blocks: readonly RecordCandidate[],
  primary: HostMenuHost | null,
): string | null {
  const end = turnRecord(blocks)
  if (end?.type !== 'error' || end.code !== 'host_offline' || !primary) {
    return null
  }
  return primary.kind === 'device' && primary.state !== 'online' ? primary.name : null
}
