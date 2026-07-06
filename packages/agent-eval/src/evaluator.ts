import type { Block } from '@demicodes/core'
import type { AgentEvalCase, InterventionChannel, InterventionType } from './case-schema'
import type { OracleResult } from './oracle'
import { assistanceScoreFor } from './scoring'

/**
 * The Evaluator is judge and supervisor in one: each check first verdicts the
 * current state from oracle evidence, and — when the run should continue —
 * produces the next structured intervention. It never edits the workspace.
 */

export interface EvaluatorDecision {
  checkIndex: number
  timestamp: string
  verdict: 'pass' | 'partial' | 'fail' | 'continue' | 'timeout'
  confidence: 'low' | 'medium' | 'high'
  evidence: Array<{ source: 'oracle' | 'transcript' | 'diff' | 'runtime'; ref: string; summary: string }>
  missingRequirements: string[]
  falseDoneDetected: boolean
  intervention?: {
    channel: InterventionChannel
    type: InterventionType
    message: string
    assistanceScore: number
    rationale: string
  }
}

export interface EvaluatorCheckContext {
  evalCase: AgentEvalCase
  checkIndex: number
  oracleResults: OracleResult[]
  transcriptBlocks: Block[]
  workerIdle: boolean
  interventionsDelivered: number
  budgetExhausted: boolean
  now(): string
}

export interface EvaluatorDriver {
  decide(context: EvaluatorCheckContext): Promise<EvaluatorDecision> | EvaluatorDecision
}

/**
 * Deterministic Evaluator for CI: verdicts purely from oracle evidence and
 * escalates through the case's intervention ladder. False done is detected
 * when the Worker went idle (implicitly claiming completion) while oracle
 * evidence still fails.
 */
export class ScriptedEvaluator implements EvaluatorDriver {
  decide(context: EvaluatorCheckContext): EvaluatorDecision {
    const failing = context.oracleResults.filter((result) => !result.passed)
    const evidence = context.oracleResults.map((result) => ({
      source: 'oracle' as const,
      ref: result.name,
      summary: result.summary,
    }))
    const base = {
      checkIndex: context.checkIndex,
      timestamp: context.now(),
      evidence,
      missingRequirements: failing.map((result) => result.summary),
      falseDoneDetected: context.workerIdle && failing.length > 0,
    }

    if (failing.length === 0) {
      return { ...base, verdict: 'pass', confidence: 'high', falseDoneDetected: false }
    }

    if (context.budgetExhausted) {
      return { ...base, verdict: 'timeout', confidence: 'high' }
    }

    const ladder = context.evalCase.evaluator.interventionPolicy.ladder
    if (ladder.length === 0 || context.interventionsDelivered >= context.evalCase.budgets.maxInterventions) {
      return { ...base, verdict: 'timeout', confidence: 'medium' }
    }
    const step = ladder[Math.min(context.interventionsDelivered, ladder.length - 1)]!
    const message =
      step.type === 'oracle_evidence'
        ? `${step.message}\n\nOracle evidence:\n${failing.map((result) => `- ${result.name}: ${result.summary}`).join('\n')}`
        : step.message
    return {
      ...base,
      verdict: 'continue',
      confidence: 'medium',
      intervention: {
        channel: step.channel,
        type: step.type,
        message,
        assistanceScore: assistanceScoreFor(step.type, step.channel),
        rationale: `oracle failing: ${failing.map((result) => result.name).join(', ')}`,
      },
    }
  }
}
