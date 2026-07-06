import { mkdtemp, readFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { events } from '@demicodes/provider/testing'
import { buildSuiteSummary, renderSuiteSummaryMarkdown, writeAttemptArtifacts, writeSuiteSummary } from '../artifacts'
import { runEvalCase } from '../runner'
import { baseCase, stubProvider } from './helpers'

test('attempt artifacts are complete enough to audit the run', async () => {
  const evalCase = baseCase()
  const providers = [
    stubProvider(() => [
      [events.text('done!'), events.response()],
      [events.toolCall('t1', 'shell_exec', { script: "printf 'done' > done.txt", timeoutMs: 10_000 }), events.response()],
      [events.text('created'), events.response()],
    ]),
  ]
  const result = await runEvalCase({ evalCase, providers })
  const runRoot = await mkdtemp(join(tmpdir(), 'demi-eval-run-'))

  const { directory } = await writeAttemptArtifacts(runRoot, evalCase, result)

  const run = JSON.parse(await readFile(join(directory, 'run.json'), 'utf8')) as Record<string, unknown>
  expect(run).toMatchObject({ caseId: evalCase.id, attempt: 1, finalStatus: 'pass' })

  const decisions = (await readFile(join(directory, 'evaluator-decisions.jsonl'), 'utf8'))
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line) as Record<string, unknown>)
  expect(decisions).toHaveLength(2)
  expect(decisions[0]).toMatchObject({ verdict: 'continue', falseDoneDetected: true })

  const transcript = JSON.parse(await readFile(join(directory, 'worker-transcript.json'), 'utf8')) as {
    blocks: Array<{ type: string }>
  }
  expect(transcript.blocks.some((block) => block.type === 'tool_call')).toBe(true)

  const oracleRound = JSON.parse(await readFile(join(directory, 'oracle', 'round-001.json'), 'utf8')) as Array<{
    passed: boolean
  }>
  expect(oracleRound[0]).toMatchObject({ passed: false })

  expect(await readFile(join(directory, 'workspace.diff'), 'utf8')).toContain('done.txt')
  expect(JSON.parse(await readFile(join(directory, 'case.json'), 'utf8'))).toMatchObject({ id: evalCase.id })

  // Suite-level summary aggregates the attempt.
  const summary = buildSuiteSummary({
    runId: 'test-run',
    suiteName: 'unit',
    passThreshold: 1,
    startedAt: result.startedAt,
    finishedAt: result.finishedAt,
    results: [{ evalCase, result }],
  })
  expect(summary.passedThreshold).toBe(true)
  expect(summary.successRates.supervisedSuccessRate).toBe(1)
  expect(summary.successRates.autonomousSuccessRate).toBe(0)

  await writeSuiteSummary(runRoot, summary)
  const markdown = await readFile(join(runRoot, 'summary.md'), 'utf8')
  expect(markdown).toContain(evalCase.id)
  expect(markdown).toContain('Pass rate: 100.0%')
  expect(renderSuiteSummaryMarkdown(summary)).toBe(markdown)
})
