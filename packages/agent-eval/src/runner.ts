import { errorMessage, waitFor } from '@demicodes/utils'
import { AgentServer, type AgentClient, type ClientSessionEvent } from '@demicodes/agent'
import type { Block, ModelSelection, SessionPhase } from '@demicodes/core'
import { createCodingAgentHarness } from '@demicodes/coding-agent'
import { LocalHost } from '@demicodes/host-local'
import { modelSelectionFromCatalog, type Provider } from '@demicodes/provider'
import type { AgentEvalCase } from './case-schema'
import { ScriptedEvaluator, type EvaluatorDecision, type EvaluatorDriver } from './evaluator'
import { computeMetrics, type EvalMetrics } from './metrics'
import { runOracles, type OracleResult } from './oracle'
import { computeFinalScore, type EvalOutcomeStatus } from './scoring'
import {
  diffWorkspace,
  prepareWorkspace,
  snapshotWorkspace,
  type WorkspaceDiffEntry,
  type WorkspaceSnapshot,
} from './workspace'

export interface RunCaseOptions {
  evalCase: AgentEvalCase
  /** Provider registry; must contain the case's worker provider id. */
  providers: Provider[]
  evaluator?: EvaluatorDriver
  attempt?: number
  now?: () => string
}

export interface EvalRunResult {
  caseId: string
  attempt: number
  startedAt: string
  finishedAt: string
  finalStatus: EvalOutcomeStatus
  finalScore: number
  metrics: EvalMetrics
  decisions: EvaluatorDecision[]
  oracleRounds: OracleResult[][]
  transcriptBlocks: Block[]
  workspace: string
  workspaceDiff: WorkspaceDiffEntry[]
}

interface ControlActionCounts {
  retry: number
  resume: number
  abort: number
  compact: number
}

/**
 * Runs one case attempt: fresh workspace, Worker driven through the public
 * AgentServer/AgentClient protocol (never a direct AgentSession), then the
 * Evaluator loop — check oracle evidence, verdict, intervene — until pass,
 * budget exhaustion, or wall timeout.
 */
export async function runEvalCase(options: RunCaseOptions): Promise<EvalRunResult> {
  const evalCase = options.evalCase
  const evaluator = options.evaluator ?? new ScriptedEvaluator()
  const now = options.now ?? (() => new Date().toISOString())
  const startedAt = now()
  const startedMs = Date.now()
  const deadlineMs = startedMs + evalCase.budgets.maxWallMs

  const workspace = await prepareWorkspace(evalCase.fixture?.source ?? null, evalCase.fixture?.ignore ?? [])
  const workspaceBefore = await snapshotWorkspace(workspace)

  const host = new LocalHost(workspace)
  const harness = createCodingAgentHarness({ host })
  const server = new AgentServer({
    agent: harness,
    providers: options.providers,
    shell: { initialEnv: { PATH: process.env.PATH ?? '' } },
  })
  const client = server.client()

  const phaseLog: SessionPhase[] = []
  let currentPhase: SessionPhase | null = null
  let providerRetries = 0
  const unsubscribe = client.subscribe((event: ClientSessionEvent) => {
    if (event.type === 'phase') {
      currentPhase = event.phase
      phaseLog.push(event.phase)
    }
    if (event.type === 'retry_scheduled') providerRetries += 1
  })

  const decisions: EvaluatorDecision[] = []
  const oracleRounds: OracleResult[][] = []
  const controlActions: ControlActionCounts = { retry: 0, resume: 0, abort: 0, compact: 0 }
  let interventionsDelivered = 0
  let finalStatus: EvalOutcomeStatus = 'timeout'
  // Captured before close(): the client clears its transcript view on 'closed'.
  let finalBlocks: Block[] = []

  try {
    await client.open(providerSelection(evalCase, options.providers), workspace, globalThis.crypto.randomUUID())
    await deliverTurn(client, evalCase.task.prompt, deadlineMs)

    for (let checkIndex = 0; decisions.length < evalCase.budgets.maxEvaluatorChecks; checkIndex += 1) {
      const workspaceAfter = await snapshotWorkspace(workspace)
      const transcriptBlocks = client.transcript().blocks
      finalBlocks = transcriptBlocks
      const oracleResults = await runOracles(evalCase.oracle, {
        workspace,
        transcriptBlocks,
        workspaceBefore,
        workspaceAfter,
      })
      oracleRounds.push(oracleResults)

      const budgetExhausted =
        Date.now() >= deadlineMs ||
        interventionsDelivered >= evalCase.budgets.maxInterventions ||
        decisions.length + 1 >= evalCase.budgets.maxEvaluatorChecks ||
        currentAssistance(decisions) >= evalCase.budgets.maxAssistanceScore

      const decision = await evaluator.decide({
        evalCase,
        checkIndex,
        oracleResults,
        transcriptBlocks,
        workerIdle: currentPhase === 'idle' || currentPhase === null,
        interventionsDelivered,
        budgetExhausted,
        now,
      })
      decisions.push(decision)

      if (decision.verdict === 'pass') {
        finalStatus = 'pass'
        break
      }
      if (decision.verdict === 'partial' || decision.verdict === 'fail' || decision.verdict === 'timeout') {
        finalStatus = decision.verdict === 'partial' ? 'partial' : decision.verdict === 'fail' ? 'fail' : 'timeout'
        break
      }

      const intervention = decision.intervention
      if (!intervention) {
        finalStatus = 'timeout'
        break
      }
      interventionsDelivered += 1
      await deliverIntervention(client, intervention.channel, intervention.message, deadlineMs, controlActions)
    }
  } finally {
    unsubscribe()
    finalBlocks = client.transcript().blocks.length > 0 ? client.transcript().blocks : finalBlocks
    try {
      await client.close()
    } catch {
      // Session teardown failures must not mask the eval result.
    }
    await server.close()
  }

  const finishedAt = now()
  const wallMs = Date.now() - startedMs
  const assistanceOverBudget = currentAssistance(decisions) > evalCase.budgets.maxAssistanceScore
  const metrics = computeMetrics({
    status: finalStatus,
    strictAssistanceThreshold: evalCase.scoring.strictAssistanceThreshold,
    transcriptBlocks: finalBlocks,
    decisions,
    deliveredControlActions: controlActions,
    providerRetries,
    wallMs,
  })
  const workspaceAfter = await snapshotWorkspace(workspace)
  return {
    caseId: evalCase.id,
    attempt: options.attempt ?? 1,
    startedAt,
    finishedAt,
    finalStatus,
    finalScore: computeFinalScore({
      status: finalStatus,
      assistanceScore: metrics.interaction.assistanceScore,
      falseDoneCount: metrics.outcome.falseDoneCount,
      overBudget: assistanceOverBudget || wallMs > evalCase.budgets.maxWallMs,
    }),
    metrics,
    decisions,
    oracleRounds,
    transcriptBlocks: finalBlocks,
    workspace,
    workspaceDiff: diffWorkspace(workspaceBefore, workspaceAfter),
  }
}

function providerSelection(evalCase: AgentEvalCase, providers: Provider[]): { providerId: string; model: ModelSelection } {
  const provider = providers.find((candidate) => candidate.id === evalCase.worker.provider)
  if (!provider) throw new Error(`Eval case "${evalCase.id}" needs provider "${evalCase.worker.provider}"`)
  const modelId = evalCase.worker.modelId ?? 'eval-default'
  const model = modelSelectionFromCatalog(provider.id, null, {
    modelId,
    thinking: evalCase.worker.thinkingEffort
      ? { type: 'effort', effort: evalCase.worker.thinkingEffort, summary: null }
      : null,
    serviceTierId: evalCase.worker.serviceTierId ?? null,
    fallbackName: modelId,
  })
  return { providerId: provider.id, model }
}

async function deliverTurn(client: AgentClient, text: string, deadlineMs: number): Promise<void> {
  await withDeadline(
    client.send([{ type: 'text', text }]),
    deadlineMs,
    'worker turn exceeded the wall budget',
  )
}

async function deliverIntervention(
  client: AgentClient,
  channel: string,
  message: string,
  deadlineMs: number,
  controlActions: ControlActionCounts,
): Promise<void> {
  switch (channel) {
    case 'send':
      await deliverTurn(client, message, deadlineMs)
      return
    case 'steer':
      try {
        await client.steer([{ type: 'text', text: message }])
      } catch {
        // No active turn to steer: deliver as a supervised follow-up instead.
        await deliverTurn(client, message, deadlineMs)
      }
      return
    case 'retry':
      controlActions.retry += 1
      await withDeadline(client.retry(), deadlineMs, 'retry exceeded the wall budget')
      return
    case 'resume':
      controlActions.resume += 1
      await withDeadline(client.resume(), deadlineMs, 'resume exceeded the wall budget')
      return
    case 'abort':
      controlActions.abort += 1
      await client.abort()
      return
    case 'compact':
      controlActions.compact += 1
      await withDeadline(client.compact(), deadlineMs, 'compact exceeded the wall budget')
      return
    default:
      throw new Error(`Unknown intervention channel: ${channel}`)
  }
}

async function withDeadline(promise: Promise<unknown>, deadlineMs: number, label: string): Promise<void> {
  let settled = false
  const guarded = promise.then(
    () => {
      settled = true
    },
    (error: unknown) => {
      settled = true
      // Worker-turn errors are part of the observed behavior, not runner crashes:
      // the Evaluator sees them through the transcript's error blocks.
      void errorMessage(error)
    },
  )
  await Promise.race([
    guarded,
    waitFor(() => settled || Date.now() >= deadlineMs, () => label, {
      timeoutMs: Math.max(1, deadlineMs - Date.now() + 1_000),
      intervalMs: 25,
    }),
  ])
}

function currentAssistance(decisions: readonly EvaluatorDecision[]): number {
  return decisions.reduce((total, decision) => total + (decision.intervention?.assistanceScore ?? 0), 0)
}
