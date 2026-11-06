import type { Block } from '@demicodes/protocol'
import { demoModel, inRequests } from './blocks'

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString()

/** A fix the agent finished, ending on a question the user answers in a few words. */
export function suggestedRepliesConversation(): Block[] {
  return inRequests([
    {
      type: 'user', id: 'suggest-question', turnId: 'suggest-turn', createdAt: minutesAgo(6), model: demoModel,
      content: [{ type: 'text', text: 'The login test fails after the session cookie rename. Fix it.' }],
      preamble: null,
    },
    {
      type: 'text', id: 'suggest-reply', createdAt: minutesAgo(5), endedAt: minutesAgo(5), model: demoModel,
      text: 'The test still read the old `sid` cookie. It reads `session` now, and the whole auth suite passes: 48 tests, none failing.\n\nShall I merge it into main?',
    },
  ])
}

/**
 * What the small model suggests after each answer of the live specimen, in
 * turn: one after the first, whose question most answers put, none after the
 * next, which puts nothing to the user, then again from the start.
 */
export const SUGGESTION_ROUNDS: readonly (readonly string[])[] = [
  ['Merge it'],
  [],
]

/** What it suggests after an answer that names the options to choose among. */
export const CHOICE_SUGGESTIONS: readonly string[] = ['Squash it', 'Rebase it', 'Keep the merge commit']

/**
 * Suggestions long enough that a narrow row wraps them, each within the
 * longest the backend keeps, the second one long word and the last wider
 * than the row.
 */
export const LONG_SUGGESTIONS: readonly string[] = [
  'Merge it, then ship a release to the beta channel for the team',
  'Rename SessionCookieCompatibilityRegressionTestHelper',
  'Rename the cookie helper so its name says which cookie it reads, and update every test that names it',
]
