import type { Block } from '@demicodes/protocol'
import type { SuggestedReply } from '@demicodes/web-ui/agent/suggested-replies'
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

const answer = (text: string): SuggestedReply => ({ text, kind: 'answer' })
const topic = (text: string): SuggestedReply => ({ text, kind: 'topic' })

/**
 * What the small model suggests after each answer of the live specimen, in
 * turn: two answers and a new topic after the first, one answer after the
 * next, none after the third, then again from the start.
 */
export const SUGGESTION_ROUNDS: readonly (readonly SuggestedReply[])[] = [
  [answer('Merge it'), answer('Show me the diff first'), topic('Add a test for the old cookie name')],
  [answer('Commit it')],
  [],
]

/** Suggestions long enough that the row narrows them, each ending in an ellipsis. */
export const LONG_SUGGESTIONS: readonly SuggestedReply[] = [
  answer('Merge it, then ship a release to the beta channel for the team'),
  answer('Run the whole auth suite again on the staging database first'),
  topic('Rename the cookie helper so its name says which cookie it reads'),
]
