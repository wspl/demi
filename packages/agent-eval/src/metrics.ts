import { zeroUsage, type Block, type TokenUsage } from '@demicodes/core'
import type { InterventionChannel, InterventionType } from './case-schema'
import type { EvaluatorDecision } from './evaluator'
import type { EvalOutcomeStatus } from './scoring'

export interface EvalMetrics {
  outcome: {
    status: EvalOutcomeStatus
    autonomousPass: boolean
    supervisedPass: boolean
    strictPass: boolean
    falseDoneCount: number
  }
  interaction: {
    workerTurns: number
    steerCount: number
    retryCount: number
    resumeCount: number
    abortCount: number
    compactCount: number
    evaluatorChecks: number
    interventionCount: number
    assistanceScore: number
    interventionByType: Partial<Record<InterventionType, number>>
    interventionByChannel: Partial<Record<InterventionChannel, number>>
  }
  worker: {
    providerRequests: number
    providerErrors: number
    providerRetries: number
    toolCalls: number
    contextCompactions: number
    wallMs: number
    usage: TokenUsage
  }
  efficiency: {
    supervisorBurdenScore: number
    interventionsToPass: number | null
    workerTurnsToPass: number | null
  }
}

export interface MetricsInputs {
  status: EvalOutcomeStatus
  strictAssistanceThreshold: number
  transcriptBlocks: Block[]
  decisions: EvaluatorDecision[]
  deliveredControlActions: { retry: number; resume: number; abort: number; compact: number }
  providerRetries: number
  wallMs: number
}

export function computeMetrics(inputs: MetricsInputs): EvalMetrics {
  const blocks = inputs.transcriptBlocks
  const interventions = inputs.decisions.filter((decision) => decision.intervention)
  const assistanceScore = interventions.reduce((total, decision) => total + (decision.intervention?.assistanceScore ?? 0), 0)
  const falseDoneCount = inputs.decisions.filter((decision) => decision.falseDoneDetected).length

  const usage = blocks.reduce((total: TokenUsage, block) => {
    if (block.type !== 'response') return total
    return {
      inputTokens: total.inputTokens + block.usage.inputTokens,
      outputTokens: total.outputTokens + block.usage.outputTokens,
      cacheReadTokens: total.cacheReadTokens + block.usage.cacheReadTokens,
      cacheWriteTokens: total.cacheWriteTokens + block.usage.cacheWriteTokens,
    }
  }, zeroUsage())

  const byType: Partial<Record<InterventionType, number>> = {}
  const byChannel: Partial<Record<InterventionChannel, number>> = {}
  for (const decision of interventions) {
    const intervention = decision.intervention!
    byType[intervention.type] = (byType[intervention.type] ?? 0) + 1
    byChannel[intervention.channel] = (byChannel[intervention.channel] ?? 0) + 1
  }

  const workerTurns = blocks.filter((block) => block.type === 'user' && !block.hidden).length
  const passed = inputs.status === 'pass'

  return {
    outcome: {
      status: inputs.status,
      autonomousPass: passed && interventions.length === 0,
      supervisedPass: passed,
      strictPass: passed && assistanceScore <= inputs.strictAssistanceThreshold,
      falseDoneCount,
    },
    interaction: {
      workerTurns,
      steerCount: blocks.filter((block) => block.type === 'steer' && !block.hidden).length,
      retryCount: inputs.deliveredControlActions.retry,
      resumeCount: inputs.deliveredControlActions.resume,
      abortCount: inputs.deliveredControlActions.abort,
      compactCount: inputs.deliveredControlActions.compact,
      evaluatorChecks: inputs.decisions.length,
      interventionCount: interventions.length,
      assistanceScore,
      interventionByType: byType,
      interventionByChannel: byChannel,
    },
    worker: {
      providerRequests: blocks.filter((block) => block.type === 'response' || block.type === 'error').length,
      providerErrors: blocks.filter((block) => block.type === 'error').length,
      providerRetries: inputs.providerRetries,
      toolCalls: blocks.filter((block) => block.type === 'tool_call').length,
      contextCompactions: blocks.filter((block) => block.type === 'compaction_boundary').length,
      wallMs: inputs.wallMs,
      usage,
    },
    efficiency: {
      supervisorBurdenScore: inputs.decisions.length + interventions.length + assistanceScore,
      interventionsToPass: passed ? interventions.length : null,
      workerTurnsToPass: passed ? workerTurns : null,
    },
  }
}
