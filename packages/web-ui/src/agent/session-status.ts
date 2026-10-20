import type { SentenceText } from '../ui/ui-text'
import type { ShownError } from '../infra/errors'
import { isCompactionDivider } from './visible-blocks'
import type { HostMenuHost } from '../hosts/types'
import { t } from './locales/text'

/** Sidebar conversation list, or a session that is not reconnecting. */
export type ListLoad = 'ready' | 'loading' | 'failed'

/**
 * How far the conversation's opening has come, and the state of its connection
 * since:
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
 * - `offline`: the opening cannot reach the backend and the page keeps no
 *   copy of the conversation for opening offline; it opens once the backend answers.
 */
export type SessionLoad = ListLoad | 'opening' | 'connecting' | 'reconnecting' | 'offline'

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
 * of a message in it, unless its opening failed.
 */
export function transcriptShown(load: SessionLoad): boolean {
  return load !== 'loading' && load !== 'failed' && load !== 'offline'
}

/** Whether the conversation is still opening for the first time: what joins it now is not new. */
export function isOpening(load: SessionLoad): boolean {
  return load === 'loading' || load === 'opening' || load === 'connecting' || load === 'offline'
}

/**
 * What the session pane shows instead of the transcript. `empty` is an open
 * conversation with no messages yet: the composer under it is the way in.
 * `none` is the chat route with no conversation open at all, so the pane
 * itself offers to start one. `offline` is a conversation the page cannot
 * read while the backend cannot be reached and keeps no copy of.
 */
export type SessionStatusKind = 'loading' | 'failed' | 'empty' | 'missing' | 'none' | 'offline'

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
  if (load === 'offline') {
    return 'offline'
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

export function sessionStatusCopy(kind: SessionStatusKind): {
  label: SentenceText
  action?: 'retry' | 'create'
} {
  if (kind === 'loading') {
    return { label: t('session.loading') }
  }
  if (kind === 'failed') {
    return {
      label: t('session.loadFailed'),
      action: 'retry',
    }
  }
  if (kind === 'missing') {
    return {
      label: t('session.notFound'),
      action: 'create',
    }
  }
  if (kind === 'none') {
    return {
      label: t('session.noneOpen'),
      action: 'create',
    }
  }
  if (kind === 'offline') {
    return { label: t('session.unavailableOffline') }
  }
  return { label: t('session.empty') }
}

/**
 * The recovery the dock offers for a turn that did not finish, from the record
 * that ended the last turn among the visible `blocks`: an error resumes, the
 * user's own Stop continues, and anything else is finished or still running. An
 * error that ended no turn, such as a Compact whose summary request failed, a
 * compaction's boundary and marker, and the agent messages and command reports
 * a failed turn took after it ended leave the turn before them as it was
 * (`failures-and-recovery.md` § The unfinished turn). Both are the session's
 * `resume`; the word tells the cause.
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
 * with, beside Resume, on an entry without rotation
 * (`crates/backend-providers/src/llm/rotation.rs`): the one its failure's facts
 * name. It reads the
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

/**
 * Whether the unfinished turn ended because the vendor signed its account
 * out, which the dock answers with Open Providers
 * (`crates/provider-claude-code/src/account.rs`): its failure's facts name the
 * account while it is
 * still signed out. False while running, and for any other end.
 */
export function signedOutOffer(
  phase: 'idle' | string,
  blocks: readonly (RecordCandidate & { id?: string })[],
  failures: Readonly<Record<string, { signedOut?: { provider: string; account: string } | null }>>,
): boolean {
  if (phase !== 'idle') {
    return false
  }
  const end = turnRecord(blocks)
  return end?.type === 'error' && !!end.id && !!failures[end.id]?.signedOut
}

/**
 * The port that the unfinished turn's request named on the user's device
 * while the device had not exposed it, which the dock offers to expose
 * before the turn resumes (`port-exposure.md` § Exposed ports, Recovery):
 * none once `exposed` says the port is exposed by now, from Settings or
 * another tab, when Resume alone tries again. Null while running, and for
 * any other end.
 */
export function notExposedOffer(
  phase: 'idle' | string,
  blocks: readonly (RecordCandidate & { id?: string })[],
  failures: Readonly<Record<string, { notExposed?: { deviceId: string; device: string; port: number } | null }>>,
  exposed: (deviceId: string, port: number) => boolean,
): { deviceId: string; device: string; port: number } | null {
  if (phase !== 'idle') {
    return null
  }
  const end = turnRecord(blocks)
  if (end?.type !== 'error' || end.code !== 'port_not_exposed' || !end.id) {
    return null
  }
  const named = failures[end.id]?.notExposed ?? null
  return named && !exposed(named.deviceId, named.port) ? named : null
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
