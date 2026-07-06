import { defineProvider, type Provider } from '@demicodes/provider'
import { StubProvider } from '@demicodes/provider/testing'
import type { AgentEvalCase } from '../case-schema'
import { parseEvalCase } from '../case-schema'

export function stubProvider(turnsFactory: () => ConstructorParameters<typeof StubProvider>[0]): Provider {
  return defineProvider({
    id: 'stub',
    displayName: 'Stub',
    createRuntime: () => new StubProvider(turnsFactory()),
  })
}

export function baseCase(overrides: Record<string, unknown> = {}): AgentEvalCase {
  return parseEvalCase({
    id: 'coding.test-case.small',
    title: 'Test case',
    category: 'coding',
    difficulty: 'small',
    worker: { provider: 'stub' },
    evaluator: {
      driver: 'scripted',
      interventionPolicy: {
        ladder: [
          { type: 'nudge', message: 'Keep going: the acceptance oracle still fails.' },
          { type: 'oracle_evidence', message: 'Still failing.' },
        ],
      },
    },
    task: {
      prompt: 'Create done.txt containing exactly "done".',
      successCriteria: ['done.txt exists with the expected content'],
    },
    budgets: { maxWallMs: 30_000, maxEvaluatorChecks: 4, maxInterventions: 2 },
    oracle: [{ type: 'file', path: 'done.txt', textIncludes: ['done'] }],
    ...overrides,
  })
}
