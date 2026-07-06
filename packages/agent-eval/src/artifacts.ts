import { mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { AgentEvalCase } from './case-schema'
import type { EvalRunResult } from './runner'
import { aggregateSuccessRates, type CaseOutcomeSummary, type SuccessRates } from './scoring'
import { renderWorkspaceDiff } from './workspace'

/**
 * Run artifacts must support post-hoc audit: why a case failed, what help the
 * Evaluator gave, and when the Worker claimed completion falsely. Layout
 * mirrors docs/internal/agent-evaluation-plan.md §10.
 */

export interface WrittenAttemptArtifacts {
  directory: string
}

export async function writeAttemptArtifacts(
  runRoot: string,
  evalCase: AgentEvalCase,
  result: EvalRunResult,
): Promise<WrittenAttemptArtifacts> {
  const directory = join(runRoot, 'cases', evalCase.id, `attempt-${String(result.attempt).padStart(2, '0')}`)
  await mkdir(join(directory, 'oracle'), { recursive: true })

  await writeJson(join(directory, 'case.json'), evalCase)
  await writeJson(join(directory, 'run.json'), {
    caseId: result.caseId,
    attempt: result.attempt,
    startedAt: result.startedAt,
    finishedAt: result.finishedAt,
    worker: evalCase.worker,
    evaluator: { driver: evalCase.evaluator.driver },
    finalStatus: result.finalStatus,
    finalScore: result.finalScore,
    metrics: result.metrics,
  })
  await writeJson(join(directory, 'metrics.json'), result.metrics)
  await writeJson(join(directory, 'worker-transcript.json'), { blocks: result.transcriptBlocks })
  await writeFile(
    join(directory, 'evaluator-decisions.jsonl'),
    result.decisions.map((decision) => JSON.stringify(decision)).join('\n') + '\n',
    'utf8',
  )
  for (const [roundIndex, round] of result.oracleRounds.entries()) {
    await writeJson(join(directory, 'oracle', `round-${String(roundIndex + 1).padStart(3, '0')}.json`), round)
  }
  await writeFile(join(directory, 'workspace.diff'), renderWorkspaceDiff(result.workspaceDiff), 'utf8')
  return { directory }
}

export interface SuiteSummary {
  runId: string
  startedAt: string
  finishedAt: string
  suiteName: string
  passThreshold: number
  passRate: number
  passedThreshold: boolean
  successRates: SuccessRates
  cases: Array<{
    caseId: string
    attempt: number
    status: EvalRunResult['finalStatus']
    score: number
    interventionCount: number
    assistanceScore: number
    falseDoneCount: number
    wallMs: number
  }>
}

export function buildSuiteSummary(options: {
  runId: string
  suiteName: string
  passThreshold: number
  startedAt: string
  finishedAt: string
  results: Array<{ evalCase: AgentEvalCase; result: EvalRunResult }>
}): SuiteSummary {
  const outcomes: CaseOutcomeSummary[] = options.results.map(({ evalCase, result }) => ({
    caseId: result.caseId,
    status: result.finalStatus,
    interventionCount: result.metrics.interaction.interventionCount,
    assistanceScore: result.metrics.interaction.assistanceScore,
    strictAssistanceThreshold: evalCase.scoring.strictAssistanceThreshold,
  }))
  const successRates = aggregateSuccessRates(outcomes)
  return {
    runId: options.runId,
    startedAt: options.startedAt,
    finishedAt: options.finishedAt,
    suiteName: options.suiteName,
    passThreshold: options.passThreshold,
    passRate: successRates.supervisedSuccessRate,
    passedThreshold: successRates.supervisedSuccessRate >= options.passThreshold,
    successRates,
    cases: options.results.map(({ result }) => ({
      caseId: result.caseId,
      attempt: result.attempt,
      status: result.finalStatus,
      score: result.finalScore,
      interventionCount: result.metrics.interaction.interventionCount,
      assistanceScore: result.metrics.interaction.assistanceScore,
      falseDoneCount: result.metrics.outcome.falseDoneCount,
      wallMs: result.metrics.worker.wallMs,
    })),
  }
}

export async function writeSuiteSummary(runRoot: string, summary: SuiteSummary): Promise<void> {
  await mkdir(runRoot, { recursive: true })
  await writeJson(join(runRoot, 'summary.json'), summary)
  await writeFile(join(runRoot, 'summary.md'), renderSuiteSummaryMarkdown(summary), 'utf8')
}

export function renderSuiteSummaryMarkdown(summary: SuiteSummary): string {
  const lines = [
    `# Eval run ${summary.runId}`,
    '',
    `- Suite: ${summary.suiteName}`,
    `- Window: ${summary.startedAt} → ${summary.finishedAt}`,
    `- Pass rate: ${(summary.passRate * 100).toFixed(1)}% (threshold ${(summary.passThreshold * 100).toFixed(0)}%: ${summary.passedThreshold ? 'met' : 'NOT met'})`,
    `- Autonomous: ${(summary.successRates.autonomousSuccessRate * 100).toFixed(1)}%, strict: ${(summary.successRates.strictSuccessRate * 100).toFixed(1)}%`,
    '',
    '| case | attempt | status | score | interventions | assistance | false done | wall ms |',
    '| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |',
  ]
  for (const entry of summary.cases) {
    lines.push(
      `| ${entry.caseId} | ${entry.attempt} | ${entry.status} | ${entry.score} | ${entry.interventionCount} | ${entry.assistanceScore} | ${entry.falseDoneCount} | ${entry.wallMs} |`,
    )
  }
  return `${lines.join('\n')}\n`
}

async function writeJson(path: string, value: unknown): Promise<void> {
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`, 'utf8')
}
