import type { InterventionChannel, InterventionType } from './case-schema'

/**
 * Scoring: interventions carry a structured assistance cost by type and
 * channel; the final score is a summary only — the raw metrics always ship
 * alongside it.
 */

export const INTERVENTION_TYPE_SCORES: Record<InterventionType, number> = {
  nudge: 2,
  oracle_evidence: 4,
  defect_report: 6,
  directional_hint: 12,
  solution_hint: 25,
  control_action: 6,
}

export const INTERVENTION_CHANNEL_SCORES: Record<InterventionChannel, number> = {
  send: 0,
  steer: 1,
  retry: 2,
  resume: 2,
  abort: 4,
  compact: 1,
}

export const OUTCOME_POINTS: Record<EvalOutcomeStatus, number> = {
  pass: 100,
  partial: 45,
  fail: 0,
  timeout: 0,
}

export type EvalOutcomeStatus = 'pass' | 'partial' | 'fail' | 'timeout'

export function assistanceScoreFor(type: InterventionType, channel: InterventionChannel): number {
  return INTERVENTION_TYPE_SCORES[type] + INTERVENTION_CHANNEL_SCORES[channel]
}

export interface ScoreInputs {
  status: EvalOutcomeStatus
  assistanceScore: number
  falseDoneCount: number
  overBudget: boolean
}

export function computeFinalScore(inputs: ScoreInputs): number {
  const overBudgetPenalty = inputs.overBudget ? 10 : 0
  return OUTCOME_POINTS[inputs.status] - inputs.assistanceScore - inputs.falseDoneCount * 8 - overBudgetPenalty
}

export interface CaseOutcomeSummary {
  caseId: string
  status: EvalOutcomeStatus
  interventionCount: number
  assistanceScore: number
  strictAssistanceThreshold: number
}

export interface SuccessRates {
  autonomousSuccessRate: number
  supervisedSuccessRate: number
  strictSuccessRate: number
  passAfterOneInterventionRate: number
  failAfterBudgetRate: number
}

export function aggregateSuccessRates(outcomes: readonly CaseOutcomeSummary[]): SuccessRates {
  const total = outcomes.length
  if (total === 0) {
    return {
      autonomousSuccessRate: 0,
      supervisedSuccessRate: 0,
      strictSuccessRate: 0,
      passAfterOneInterventionRate: 0,
      failAfterBudgetRate: 0,
    }
  }
  const count = (predicate: (outcome: CaseOutcomeSummary) => boolean): number => outcomes.filter(predicate).length
  return {
    autonomousSuccessRate: count((o) => o.status === 'pass' && o.interventionCount === 0) / total,
    supervisedSuccessRate: count((o) => o.status === 'pass') / total,
    strictSuccessRate: count((o) => o.status === 'pass' && o.assistanceScore <= o.strictAssistanceThreshold) / total,
    passAfterOneInterventionRate: count((o) => o.status === 'pass' && o.interventionCount === 1) / total,
    failAfterBudgetRate: count((o) => o.status === 'timeout' || o.status === 'fail') / total,
  }
}
