import { expect, test } from 'bun:test'
import { EvalCaseError, parseEvalCase, parseEvalSuite } from '../case-schema'
import { baseCase } from './helpers'

test('a minimal case parses with budget and scoring defaults applied', () => {
  const parsed = baseCase()
  expect(parsed.budgets.maxProviderRequests).toBe(60)
  expect(parsed.budgets.maxAssistanceScore).toBe(40)
  expect(parsed.scoring.strictAssistanceThreshold).toBe(8)
  expect(parsed.evaluator.driver).toBe('scripted')
  expect(parsed.evaluator.interventionPolicy.ladder[0]?.channel).toBe('send')
})

test('invalid oracle specs are rejected', () => {
  expect(() =>
    baseCase({ oracle: [{ type: 'command', name: 'x', command: [], timeoutMs: 1000 }] }),
  ).toThrow(EvalCaseError)
  expect(() => baseCase({ oracle: [{ type: 'mystery' }] })).toThrow(EvalCaseError)
  expect(() => baseCase({ oracle: [] })).toThrow(EvalCaseError)
})

test('solution hints must be explicitly allowed by the case policy', () => {
  expect(() =>
    baseCase({
      evaluator: {
        driver: 'scripted',
        interventionPolicy: { ladder: [{ type: 'solution_hint', message: 'change line 3 to return a+b' }] },
      },
    }),
  ).toThrow(/allowSolutionHint/)

  const allowed = baseCase({
    evaluator: {
      driver: 'scripted',
      interventionPolicy: {
        allowSolutionHint: true,
        ladder: [{ type: 'solution_hint', message: 'change line 3 to return a+b' }],
      },
    },
  })
  expect(allowed.evaluator.interventionPolicy.allowSolutionHint).toBe(true)
})

test('case ids must be dot-separated kebab segments', () => {
  expect(() => baseCase({ id: 'Bad Case Id' })).toThrow(EvalCaseError)
})

test('suites validate name, case list, and threshold bounds', () => {
  const suite = parseEvalSuite({ name: 'core', cases: ['a.json'], passThreshold: 0.5 })
  expect(suite.passThreshold).toBe(0.5)
  expect(() => parseEvalSuite({ name: 'core', cases: [] })).toThrow(EvalCaseError)
  expect(() => parseEvalSuite({ name: 'core', cases: ['a.json'], passThreshold: 2 })).toThrow(EvalCaseError)
})
