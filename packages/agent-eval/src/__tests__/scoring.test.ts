import { expect, test } from 'bun:test'
import { aggregateSuccessRates, assistanceScoreFor, computeFinalScore } from '../scoring'

test('assistance scores combine intervention type and channel costs', () => {
  expect(assistanceScoreFor('nudge', 'send')).toBe(2)
  expect(assistanceScoreFor('nudge', 'steer')).toBe(3)
  expect(assistanceScoreFor('directional_hint', 'send')).toBe(12)
  expect(assistanceScoreFor('solution_hint', 'send')).toBe(25)
  expect(assistanceScoreFor('control_action', 'abort')).toBe(10)
})

test('final score subtracts assistance, false dones, and budget overruns', () => {
  expect(computeFinalScore({ status: 'pass', assistanceScore: 0, falseDoneCount: 0, overBudget: false })).toBe(100)
  expect(computeFinalScore({ status: 'pass', assistanceScore: 6, falseDoneCount: 1, overBudget: false })).toBe(86)
  expect(computeFinalScore({ status: 'partial', assistanceScore: 2, falseDoneCount: 0, overBudget: true })).toBe(33)
  expect(computeFinalScore({ status: 'timeout', assistanceScore: 10, falseDoneCount: 2, overBudget: false })).toBe(-26)
})

test('success rates separate autonomous, supervised, and strict passes', () => {
  const rates = aggregateSuccessRates([
    { caseId: 'a', status: 'pass', interventionCount: 0, assistanceScore: 0, strictAssistanceThreshold: 8 },
    { caseId: 'b', status: 'pass', interventionCount: 1, assistanceScore: 2, strictAssistanceThreshold: 8 },
    { caseId: 'c', status: 'pass', interventionCount: 3, assistanceScore: 30, strictAssistanceThreshold: 8 },
    { caseId: 'd', status: 'timeout', interventionCount: 2, assistanceScore: 10, strictAssistanceThreshold: 8 },
  ])
  expect(rates.autonomousSuccessRate).toBe(0.25)
  expect(rates.supervisedSuccessRate).toBe(0.75)
  expect(rates.strictSuccessRate).toBe(0.5)
  expect(rates.passAfterOneInterventionRate).toBe(0.25)
  expect(rates.failAfterBudgetRate).toBe(0.25)
})
