import { readFile } from 'node:fs/promises'
import { z } from 'zod'

/**
 * Eval case contract. A case is a versionable file describing the task, the
 * Worker configuration, the Evaluator supervision policy, oracle evidence
 * sources, and budgets. Schema mirrors docs/internal/agent-evaluation-plan.md.
 */

export type InterventionChannel = 'send' | 'steer' | 'retry' | 'resume' | 'abort' | 'compact'

export type InterventionType =
  | 'nudge'
  | 'oracle_evidence'
  | 'defect_report'
  | 'directional_hint'
  | 'solution_hint'
  | 'control_action'

const transcriptAssertionSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('tool_call'), toolName: z.string().optional(), minCount: z.number().int().positive().default(1) }),
  z.object({ kind: z.literal('steer'), minCount: z.number().int().positive().default(1) }),
  z.object({ kind: z.literal('abort'), minCount: z.number().int().positive().default(1) }),
  z.object({ kind: z.literal('compaction'), minCount: z.number().int().positive().default(1) }),
  z.object({ kind: z.literal('assistant_text_includes'), text: z.string().min(1) }),
])

const oracleSchema = z.discriminatedUnion('type', [
  z.object({
    type: z.literal('command'),
    name: z.string().min(1),
    command: z.array(z.string().min(1)).min(1),
    cwd: z.string().optional(),
    timeoutMs: z.number().int().positive().max(600_000),
    expectedExitCode: z.number().int().default(0),
    stdoutIncludes: z.array(z.string()).optional(),
    stderrExcludes: z.array(z.string()).optional(),
  }),
  z.object({
    type: z.literal('file'),
    path: z.string().min(1),
    mustExist: z.boolean().default(true),
    textIncludes: z.array(z.string()).optional(),
  }),
  z.object({
    type: z.literal('transcript'),
    assertions: z.array(transcriptAssertionSchema).min(1),
  }),
  z.object({
    type: z.literal('diff'),
    allowedPaths: z.array(z.string()).optional(),
    forbiddenPaths: z.array(z.string()).optional(),
  }),
])

const interventionStepSchema = z.object({
  type: z.enum(['nudge', 'oracle_evidence', 'defect_report', 'directional_hint', 'solution_hint', 'control_action']),
  message: z.string().min(1),
  channel: z.enum(['send', 'steer', 'retry', 'resume', 'abort', 'compact']).default('send'),
})

const caseSchema = z.object({
  id: z.string().regex(/^[a-z0-9-]+(\.[a-z0-9-]+)*$/, 'case id must be dot-separated kebab segments'),
  title: z.string().min(1),
  category: z.enum(['coding', 'debugging', 'shell-control', 'long-context', 'steer-queue', 'web', 'refactor']),
  difficulty: z.enum(['small', 'medium', 'large']),
  fixture: z
    .object({
      source: z.string().min(1),
      copyMode: z.literal('fresh-copy').default('fresh-copy'),
      ignore: z.array(z.string()).default([]),
    })
    .optional(),
  worker: z.object({
    provider: z.enum(['stub', 'claude-code', 'codex', 'openai', 'anthropic']),
    modelId: z.string().optional(),
    thinkingEffort: z.string().nullable().optional(),
    serviceTierId: z.string().nullable().optional(),
  }),
  evaluator: z.object({
    driver: z.enum(['scripted', 'llm', 'human-recorded']).default('scripted'),
    rubric: z.array(z.string()).default([]),
    interventionPolicy: z.object({
      ladder: z.array(interventionStepSchema).default([]),
      allowSolutionHint: z.boolean().default(false),
    }),
  }),
  task: z.object({
    prompt: z.string().min(1),
    successCriteria: z.array(z.string().min(1)).min(1),
    explicitNonGoals: z.array(z.string()).default([]),
  }),
  budgets: z
    .object({
      maxWallMs: z.number().int().positive().default(300_000),
      maxWorkerTurns: z.number().int().positive().default(6),
      maxProviderRequests: z.number().int().positive().default(60),
      maxToolCalls: z.number().int().positive().default(80),
      maxEvaluatorChecks: z.number().int().positive().default(8),
      maxInterventions: z.number().int().positive().default(4),
      maxAssistanceScore: z.number().int().nonnegative().default(40),
    })
    .default(() => ({
      maxWallMs: 300_000,
      maxWorkerTurns: 6,
      maxProviderRequests: 60,
      maxToolCalls: 80,
      maxEvaluatorChecks: 8,
      maxInterventions: 4,
      maxAssistanceScore: 40,
    })),
  /** Real-provider cases gate on an env flag so they never run in default CI. */
  gate: z.object({ env: z.string().min(1) }).optional(),
  oracle: z.array(oracleSchema).min(1),
  scoring: z
    .object({
      strictAssistanceThreshold: z.number().int().nonnegative().default(8),
    })
    .default(() => ({ strictAssistanceThreshold: 8 })),
})

export type AgentEvalCase = z.infer<typeof caseSchema>
export type OracleSpec = AgentEvalCase['oracle'][number]
export type TranscriptAssertion = z.infer<typeof transcriptAssertionSchema>
export type InterventionStep = z.infer<typeof interventionStepSchema>

export class EvalCaseError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'EvalCaseError'
  }
}

export function parseEvalCase(value: unknown): AgentEvalCase {
  const result = caseSchema.safeParse(value)
  if (!result.success) {
    const issue = result.error.issues[0]
    throw new EvalCaseError(`Invalid eval case: ${issue?.path.join('.') ?? ''} ${issue?.message ?? 'validation failed'}`)
  }
  const parsed = result.data
  if (parsed.evaluator.interventionPolicy.ladder.some((step) => step.type === 'solution_hint') && !parsed.evaluator.interventionPolicy.allowSolutionHint) {
    throw new EvalCaseError('Invalid eval case: solution_hint interventions require allowSolutionHint: true')
  }
  return parsed
}

export async function loadEvalCase(path: string): Promise<AgentEvalCase> {
  let raw: string
  try {
    raw = await readFile(path, 'utf8')
  } catch (error) {
    throw new EvalCaseError(`Cannot read eval case ${path}: ${error instanceof Error ? error.message : String(error)}`)
  }
  let json: unknown
  try {
    json = JSON.parse(raw)
  } catch (error) {
    throw new EvalCaseError(`Eval case ${path} is not valid JSON: ${error instanceof Error ? error.message : String(error)}`)
  }
  return parseEvalCase(json)
}

const suiteSchema = z.object({
  name: z.string().min(1),
  /** Case file paths, relative to the suite file. */
  cases: z.array(z.string().min(1)).min(1),
  passThreshold: z.number().min(0).max(1).default(1),
})

export type EvalSuite = z.infer<typeof suiteSchema>

export function parseEvalSuite(value: unknown): EvalSuite {
  const result = suiteSchema.safeParse(value)
  if (!result.success) {
    const issue = result.error.issues[0]
    throw new EvalCaseError(`Invalid eval suite: ${issue?.path.join('.') ?? ''} ${issue?.message ?? 'validation failed'}`)
  }
  return result.data
}
