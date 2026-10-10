import { expect, test } from 'bun:test'
import type { ConversationSummary, LastTurn } from '../api/generated/web-api'
import { conversationSummary as summary } from '../__tests__/product-state'
import { answerStart, follow, type Followed, type NotificationSettings } from './notifications'

// Cost: pure functions over summaries; under a millisecond.

const ID = '00000000-0000-4000-8000-000000000001'
const OTHER = '00000000-0000-4000-8000-000000000002'
const ON: NotificationSettings = { enabled: true, turnFinishes: true, turnFails: true, needsPermission: true }

function ended(id: string, outcome: LastTurn['outcome'], answer: string | null = null): LastTurn {
  return { id, outcome, answerStart: answer }
}
const first = ended('r1', 'finished', 'The first answer')
const answered = summary(ID, 'Fix the login test', { status: 'completed', revision: 4, lastTurn: first })

/** What the page holds after following `summaries` in order. */
function followed(...summaries: ConversationSummary[]): Followed | undefined {
  let held: Followed | undefined
  for (const next of summaries) {
    held = follow(held, next, ON, null).followed
  }
  return held
}

/** What `next` notifies of after the page followed `before`. */
function causes(before: ConversationSummary[], next: ConversationSummary, settings = ON, front: string | null = null) {
  return follow(followed(...before), next, settings, front).causes
}

test('a turn that starts and ends between two summaries the page receives still notifies', () => {
  const second = ended('r2', 'finished', 'Fixed.')
  expect(causes([answered], { ...answered, lastTurn: second })).toEqual([{ kind: 'turnFinished', turn: second }])
  const failed = ended('e2', 'failed')
  expect(causes([answered], { ...answered, status: 'error', lastTurn: failed }, ON, OTHER)).toEqual([{ kind: 'turnFailed', turn: failed }])
  // In front of the user, nothing notifies.
  expect(causes([answered], { ...answered, lastTurn: second }, ON, ID)).toEqual([])
})

test('a turn notifies once, and a turn the user stopped notifies nothing', () => {
  const second = { ...answered, lastTurn: ended('r2', 'finished') }
  expect(causes([answered, second], { ...second, revision: 5 })).toEqual([])
  expect(causes([answered], { ...answered, status: 'stopped', lastTurn: ended('a2', 'stopped') })).toEqual([])
  expect(causes([answered], { ...answered, status: 'interrupted' })).toEqual([])
})

test('while the conversation runs, an older ended turn a retry left notifies nothing, and the turn that ends after it does', () => {
  const failed = { ...answered, status: 'error' as const, lastTurn: ended('e2', 'failed') }
  // The retry cut the failed turn from the history: the latest ended turn is the first again.
  const retrying = { ...failed, status: 'running' as const, lastTurn: first }
  expect(causes([answered, failed], retrying)).toEqual([])
  const retried = ended('r3', 'finished', 'Fixed on the second try.')
  expect(causes([answered, failed, retrying], { ...retrying, status: 'completed', lastTurn: retried })).toEqual([
    { kind: 'turnFinished', turn: retried },
  ])
})

test('nothing notifies while notifications are off, or of what the user switched off', () => {
  const next = { ...answered, lastTurn: ended('r2', 'finished') }
  expect(causes([answered], next, { ...ON, enabled: false })).toEqual([])
  expect(causes([answered], next, { ...ON, turnFinishes: false })).toEqual([])
  expect(causes([answered], { ...answered, status: 'error', lastTurn: ended('e2', 'failed') }, { ...ON, turnFails: false })).toEqual([])
})

test('a conversation the page sees for the first time tells no change, as after a reload', () => {
  expect(causes([], answered)).toEqual([])
  expect(causes([answered], { ...answered, revision: 5 })).toEqual([])
})

test('a new or replaced permission request asks for a read of the requests; a decision that leaves none does not', () => {
  const idle = summary(ID, 'Skills', { permissionsRevision: 3 })
  const waiting = { ...idle, permissionRequests: 1, permissionsRevision: 4 }
  expect(causes([idle], waiting)).toEqual([{ kind: 'permission' }])
  expect(causes([waiting], { ...waiting, permissionsRevision: 5 })).toEqual([{ kind: 'permission' }])
  expect(causes([waiting], { ...waiting, permissionRequests: 0, permissionsRevision: 5 })).toEqual([])
  expect(causes([idle], waiting, { ...ON, needsPermission: false })).toEqual([])
})

test('a finished turn says the start of its answer as plain text, cut to 120 characters', () => {
  const answer = `**Fixed.** The \`login\` test waited for the cookie; it now waits for the redirect. ${'More detail. '.repeat(20)}`
  const start = answerStart(ended('r2', 'finished', answer))
  expect(start.startsWith('Fixed. The login test waited for the cookie; it now waits for the redirect.')).toBe(true)
  expect(start).toHaveLength(120)
  expect(start.endsWith('…')).toBe(true)
  // Emphasis beside CJK punctuation is emphasis too, never stars in the text.
  expect(answerStart(ended('r2', 'finished', '**建议：**一组'))).toBe('建议：一组')
  // A turn that ended without text says nothing of an answer.
  expect(answerStart(ended('r2', 'finished'))).toBe('')
})
