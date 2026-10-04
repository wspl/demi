import type { Block } from '@demicodes/protocol'
import { demoModel } from './blocks'

/** A compaction as the transcript shows it: running, finished, or failed. */
export type CompactionCase = 'running' | 'done' | 'failed'

const createdAt = '2026-09-24T12:00:00.000Z'

function user(id: string, text: string): Block {
  return { type: 'user', id, turnId: id, createdAt, model: demoModel, content: [{ type: 'text', text }], preamble: null }
}

function answer(id: string, text: string): Block[] {
  return [
    { type: 'text', id, createdAt, model: demoModel, text, forkable: true },
    {
      type: 'response',
      id: `${id}-usage`,
      createdAt,
      model: demoModel,
      usage: { inputTokens: 120_000, outputTokens: 800, cacheReadTokens: 0, cacheWriteTokens: 0 },
    },
  ]
}

const FIRST = user('compaction-u1', 'Why does the login test fail after the cookie rename?')
const FIRST_ANSWER = answer('compaction-a1', 'The test still reads `sid`; the helper already writes `session`.')
const SECOND = user('compaction-u2', 'Update the assertion and leave the helper alone.')
const SECOND_ANSWER = answer('compaction-a2', 'Updated `auth.test.ts` to expect `session`. `cookie.ts` is unchanged.')

/**
 * The user compacts after the second answer. The pass inserts its summary
 * before that answer, where the request it summarized ended, and appends the
 * marker at the end, where the divider shows; a running pass shows there
 * too, and a failed one leaves its error record there.
 */
export function compactionTranscript(state: CompactionCase): Block[] {
  const history = [FIRST, ...FIRST_ANSWER, SECOND]
  if (state === 'running') {
    return [...history, ...SECOND_ANSWER]
  }
  if (state === 'failed') {
    return failedCompactionTranscript(true)
  }
  return [
    ...history,
    {
      type: 'compaction_boundary',
      id: 'compaction-boundary',
      createdAt,
      model: demoModel,
      summary: 'The login test expected the old cookie name; the assertion was to be updated.',
      summaryTokens: 2_400,
    },
    ...SECOND_ANSWER,
    {
      type: 'compaction_marker',
      id: 'compaction-marker',
      createdAt,
      model: demoModel,
      boundaryId: 'compaction-boundary',
      compactedTokens: 121_000,
    },
  ]
}

/**
 * The summary request failed after the second answer. `outsideTurn`: the
 * user's Compact ran the pass, so no turn ended and the dock offers no
 * Resume; otherwise the turn's own pass failed, and Resume finishes it.
 */
export function failedCompactionTranscript(outsideTurn: boolean): Block[] {
  return [
    FIRST,
    ...FIRST_ANSWER,
    SECOND,
    ...SECOND_ANSWER,
    {
      type: 'error',
      id: 'compaction-failed',
      createdAt,
      model: demoModel,
      message: 'Anthropic API request failed with HTTP 401: invalid x-api-key',
      code: 'auth_expired',
      outsideTurn,
    },
  ]
}

/**
 * The second message failed before any answer, and the user then compacted.
 * The pass summarized the first message, so its boundary sits before the
 * first answer, and the marker after the failure is the last block: the
 * failed turn behind it still offers Resume.
 */
export function compactedAfterFailedTurnTranscript(): Block[] {
  return [
    FIRST,
    {
      type: 'compaction_boundary',
      id: 'compaction-after-failure-boundary',
      createdAt,
      model: demoModel,
      summary: 'The login test expected the old cookie name.',
      summaryTokens: 1_900,
    },
    ...FIRST_ANSWER,
    SECOND,
    {
      type: 'error',
      id: 'compaction-after-failure-error',
      createdAt,
      model: demoModel,
      message: 'Anthropic API request failed with HTTP 529: Overloaded.',
      code: 'overloaded',
    },
    {
      type: 'compaction_marker',
      id: 'compaction-after-failure-marker',
      createdAt,
      model: demoModel,
      boundaryId: 'compaction-after-failure-boundary',
      compactedTokens: 60_000,
    },
  ]
}

/**
 * The user pressed Compact after the second answer and stopped it while the
 * summary was written. The stop wrote nothing, so the transcript is the
 * finished conversation it was.
 */
export function stoppedCompactTranscript(): Block[] {
  return [FIRST, ...FIRST_ANSWER, SECOND, ...SECOND_ANSWER]
}
