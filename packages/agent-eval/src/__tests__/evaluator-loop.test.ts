import { expect, test } from 'bun:test'
import { events } from '@demicodes/provider/testing'
import { runEvalCase } from '../runner'
import { baseCase, stubProvider } from './helpers'

const WRITE_SCRIPT = "printf 'done' > done.txt"

test('autonomous pass: worker completes with zero interventions', async () => {
  const providers = [
    stubProvider(() => [
      [events.toolCall('t1', 'shell_exec', { script: WRITE_SCRIPT, timeoutMs: 10_000 }), events.response()],
      [events.text('done.txt is created'), events.response()],
    ]),
  ]

  const result = await runEvalCase({ evalCase: baseCase(), providers })

  expect(result.finalStatus).toBe('pass')
  expect(result.metrics.outcome.autonomousPass).toBe(true)
  expect(result.metrics.outcome.strictPass).toBe(true)
  expect(result.metrics.interaction.interventionCount).toBe(0)
  expect(result.metrics.interaction.assistanceScore).toBe(0)
  expect(result.metrics.outcome.falseDoneCount).toBe(0)
  expect(result.finalScore).toBe(100)
  expect(result.metrics.worker.toolCalls).toBe(1)
  expect(result.decisions).toHaveLength(1)
  expect(result.decisions[0]).toMatchObject({ verdict: 'pass', falseDoneDetected: false })
})

test('false done: worker claims completion, evaluator detects it and nudges to a supervised pass', async () => {
  const providers = [
    stubProvider(() => [
      // Turn 1: claims completion without doing the work.
      [events.text('All finished. done.txt has been created.'), events.response()],
      // Turn 2 (after the nudge): actually does it.
      [events.toolCall('t2', 'shell_exec', { script: WRITE_SCRIPT, timeoutMs: 10_000 }), events.response()],
      [events.text('now it is really created'), events.response()],
    ]),
  ]

  const result = await runEvalCase({ evalCase: baseCase(), providers })

  expect(result.finalStatus).toBe('pass')
  expect(result.metrics.outcome.autonomousPass).toBe(false)
  expect(result.metrics.outcome.supervisedPass).toBe(true)
  expect(result.metrics.outcome.falseDoneCount).toBe(1)
  expect(result.metrics.interaction.interventionCount).toBe(1)
  expect(result.metrics.interaction.interventionByType.nudge).toBe(1)
  // nudge (2) + send channel (0); minus false done penalty (8).
  expect(result.metrics.interaction.assistanceScore).toBe(2)
  expect(result.finalScore).toBe(100 - 2 - 8)
  expect(result.decisions[0]).toMatchObject({ verdict: 'continue', falseDoneDetected: true })
  expect(result.decisions[0]!.intervention).toMatchObject({ type: 'nudge', channel: 'send' })
  expect(result.decisions[1]).toMatchObject({ verdict: 'pass' })
  // Two visible user turns: the task plus one supervised follow-up.
  expect(result.metrics.interaction.workerTurns).toBe(2)
})

test('oracle_evidence interventions carry the raw failing evidence', async () => {
  const providers = [
    stubProvider(() => [
      [events.text('done'), events.response()],
      [events.text('still not doing it'), events.response()],
      [events.toolCall('t3', 'shell_exec', { script: WRITE_SCRIPT, timeoutMs: 10_000 }), events.response()],
      [events.text('created'), events.response()],
    ]),
  ]

  const result = await runEvalCase({ evalCase: baseCase(), providers })

  expect(result.finalStatus).toBe('pass')
  expect(result.metrics.interaction.interventionCount).toBe(2)
  const second = result.decisions[1]!.intervention!
  expect(second.type).toBe('oracle_evidence')
  expect(second.message).toContain('Oracle evidence:')
  expect(second.message).toContain('done.txt')
  expect(result.metrics.outcome.falseDoneCount).toBe(2)
})

test('budget exhaustion finalizes as timeout with the interventions on record', async () => {
  const providers = [
    stubProvider(() => [
      [events.text('nope'), events.response()],
      [events.text('still nope'), events.response()],
      [events.text('never'), events.response()],
      [events.text('not happening'), events.response()],
    ]),
  ]

  const result = await runEvalCase({ evalCase: baseCase(), providers })

  expect(result.finalStatus).toBe('timeout')
  expect(result.metrics.outcome.supervisedPass).toBe(false)
  // maxInterventions = 2 were spent before the evaluator gave up.
  expect(result.metrics.interaction.interventionCount).toBe(2)
  expect(result.decisions.at(-1)).toMatchObject({ verdict: 'timeout' })
  expect(result.finalScore).toBeLessThan(0)
})

test('the worker runs against an isolated workspace and the diff is captured', async () => {
  const providers = [
    stubProvider(() => [
      [events.toolCall('t1', 'shell_exec', { script: WRITE_SCRIPT, timeoutMs: 10_000 }), events.response()],
      [events.text('created'), events.response()],
    ]),
  ]

  const result = await runEvalCase({ evalCase: baseCase(), providers })

  expect(result.workspaceDiff).toEqual([{ path: 'done.txt', status: 'added' }])
  expect(result.workspace).toContain('demi-eval-workspace-')
})
