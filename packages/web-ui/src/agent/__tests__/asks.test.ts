import { expect, test } from 'bun:test'
import type { Block } from '@demicodes/protocol'
import { EMPTY_TRANSCRIPT, heldTranscript } from '@demicodes/conversation-client'
import { cardContent, pageToRead, shownTurns, type ShownSideQuestion } from '../comments/asks'
import { model } from './agent-harness'

// Cost: pure functions; microseconds.

const ASKED = '2026-10-18T07:00:00.000Z'
const before = '2026-10-18T06:50:00.000Z'
const after = (seconds: number) => new Date(Date.parse(ASKED) + seconds * 1000).toISOString()

/** A user block; the side question's first message's turn is the side question's id. */
function user(id: string, createdAt: string, text: string, turnId = id): Block {
  return { type: 'user', id, turnId, createdAt, model, content: [{ type: 'text', text }], preamble: null }
}

function text(id: string, createdAt: string, words: string): Block {
  return { type: 'text', id, createdAt, model, text: words }
}

function call(id: string, createdAt: string, description: string): Block {
  return {
    type: 'tool_call', id, createdAt, model, toolUseId: `${id}-use`, toolName: 'shell', status: 'executing',
    input: JSON.stringify({ script: 'rg timeout', description }), output: [], view: null, endedAt: null,
  }
}

/** The side question holding `blocks`, the page of its transcript from `start` of `length` blocks. */
function question(blocks: Block[], working: boolean, start = 0, length = start + blocks.length): ShownSideQuestion {
  return {
    id: 'side', blockId: 'reply', start: 0, end: 5, quote: 'stale', question: 'Why stale?', title: null, folded: false,
    askedAt: ASKED, history: heldTranscript(length, [{ start, blocks }]), failures: {}, working,
  }
}

const FIRST = '> stale\n\nThe user asks this aside…\n\nWhy stale?'

/** The history a side question starts from: the conversation's, older than the question. */
const history = [user('root-q', before, 'Why is the card wrong?'), text('reply', before, 'The stale cache keeps old totals.')]

// The card shows each of its own questions with the last text its turn
// wrote, the first as the user typed it rather than with the quote and the
// instruction its message carries; the history it started from shows nothing.
test('a card shows its own questions and answers, never the history it started from', () => {
  const card = cardContent(question([
    ...history,
    user('q1', after(1), FIRST, 'side'),
    call('c1', after(2), 'Find the cache lifetime'),
    text('t1', after(3), 'Let me check.'),
    call('c2', after(4), 'Read the cache'),
    text('a1', after(5), 'Its TTL is an hour.'),
    user('q2', after(6), 'Can it be shorter?'),
    text('a2', after(7), 'Yes, a minute.'),
  ], false))
  expect(card.turns).toEqual([
    { question: 'Why stale?', status: null, answer: 'Its TTL is an hour.', streaming: false },
    { question: 'Can it be shorter?', status: null, answer: 'Yes, a minute.', streaming: false },
  ])
  expect(card.failure).toBeNull()
})

// While the last turn works, its one status line names the call it runs,
// or says it thinks before it runs any; once it writes, its text streams.
test('a working turn shows one status line until its answer starts', () => {
  const asked = [...history, user('q1', after(1), FIRST, 'side')]
  expect(cardContent(question(asked, true)).turns.at(-1)!.status).toBe('Thinking…')
  expect(cardContent(question([...asked, call('c1', after(2), 'Find the cache lifetime')], true)).turns.at(-1)!.status)
    .toBe('Find the cache lifetime')
  expect(cardContent(question([...asked, text('a1', after(3), 'Its TTL')], true)).turns.at(-1))
    .toEqual({ question: 'Why stale?', status: null, answer: 'Its TTL', streaming: true })
})

// A turn that ended with a failure offers its record, for Retry.
test('a failed turn offers its record', () => {
  const failure: Block = { type: 'error', id: 'e1', createdAt: after(3), model, message: 'Overloaded', code: 'overloaded' }
  const card = cardContent(question([...history, user('q1', after(1), FIRST, 'side'), failure], false))
  expect(card.failure?.id).toBe('e1')
  expect(cardContent(question([...history, user('q1', after(1), FIRST, 'side'), failure], true)).failure).toBeNull()
})

// A long side conversation: its first question, many steps, then
// follow-ups. The page holds only the latest page, which starts at a
// follow-up; the card does not take that follow-up for the first question
// and reads the pages before until it holds the first message, which its
// turn names, and then shows every question with its answer.
test('a card reads back until it holds its first question, whatever the pages hold', () => {
  const steps = Array.from({ length: 40 }, (_, index) => call(`c${index}`, after(2), `Step ${index}`))
  const whole = [
    ...history,
    user('q1', after(1), FIRST, 'side'),
    ...steps,
    text('a1', after(3), 'Its TTL is an hour.'),
    user('q2', after(4), 'Can it be shorter?'),
    text('a2', after(5), 'Yes, a minute.'),
    user('q3', after(6), 'And the totals?'),
    text('a3', after(7), 'They refresh with it.'),
  ]
  const latestFrom = whole.findIndex((block) => block.id === 'q3')
  const latest = question(whole.slice(latestFrom), false, latestFrom, whole.length)
  expect(pageToRead(latest)).toBe('before')
  expect(cardContent(latest).turns.map((turn) => turn.question)).toEqual(['Why stale?'])

  const middleFrom = whole.findIndex((block) => block.id === 'q2')
  const more = question(whole.slice(middleFrom), false, middleFrom, whole.length)
  expect(pageToRead(more)).toBe('before')

  const all = question(whole, false)
  expect(pageToRead(all)).toBeNull()
  expect(cardContent(all).turns.map((turn) => [turn.question, turn.answer])).toEqual([
    ['Why stale?', 'Its TTL is an hour.'],
    ['Can it be shorter?', 'Yes, a minute.'],
    ['And the totals?', 'They refresh with it.'],
  ])
  // None held yet: the latest page, unless the question is on its way.
  expect(pageToRead({ ...question([], false), history: EMPTY_TRANSCRIPT })).toBe('latest')
  expect(pageToRead({ ...question([], true), history: EMPTY_TRANSCRIPT })).toBeNull()
})

// The full window shows the side question's own turns only, its first
// message as the user's question under its quote, without the history it
// started from or the instruction.
test('the full window shows the side questions own turns, the first as the question with its quote', () => {
  const turns = shownTurns(question([...history, user('q1', after(1), FIRST, 'side'), text('a1', after(2), 'Its TTL is an hour.')], false))
  expect(turns.map((block) => block.id)).toEqual(['q1', 'a1'])
  const first = turns[0]!
  expect(first.type === 'user' && first.content).toEqual([{ type: 'text', text: '> stale\n\nWhy stale?' }])
})
