import { readFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { errorMessage } from '@demicodes/utils'
import type { Provider } from '@demicodes/provider'
import { createAnthropicApiProvider } from '@demicodes/provider-anthropic-api'
import { createClaudeCodeProvider } from '@demicodes/provider-claude-code'
import { createCodexProvider } from '@demicodes/provider-codex'
import { createOpenAIApiProvider } from '@demicodes/provider-openai-api'
import { buildSuiteSummary, writeAttemptArtifacts, writeSuiteSummary } from './artifacts'
import { EvalCaseError, loadEvalCase, parseEvalSuite, type AgentEvalCase } from './case-schema'
import { runEvalCase, type EvalRunResult } from './runner'

/**
 * Exit codes (docs/internal/agent-evaluation-plan.md §11):
 * 0 pass-threshold met · 1 runner/system error · 2 below threshold ·
 * 3 invalid case schema · 4 gated provider not enabled.
 */
export async function runCli(argv: string[]): Promise<number> {
  try {
    const [command, ...rest] = argv
    if (command === 'run') return await runCommand(rest)
    if (command === 'report') return await reportCommand(rest)
    printUsage()
    return 1
  } catch (error) {
    if (error instanceof EvalCaseError) {
      process.stderr.write(`${error.message}\n`)
      return 3
    }
    process.stderr.write(`agent-eval failed: ${errorMessage(error)}\n`)
    return 1
  }
}

async function runCommand(args: string[]): Promise<number> {
  let casePath: string | null = null
  let suitePath: string | null = null
  let attempts = 1
  let runRoot = resolve('eval-runs', new Date().toISOString().replace(/[:.]/g, '').replace('T', 'T').slice(0, 16) + 'Z')

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index]
    if (arg === '--case') casePath = requiredValue(args, ++index, '--case')
    else if (arg === '--suite') suitePath = requiredValue(args, ++index, '--suite')
    else if (arg === '--attempts') attempts = Number.parseInt(requiredValue(args, ++index, '--attempts'), 10)
    else if (arg === '--out') runRoot = resolve(requiredValue(args, ++index, '--out'))
    else throw new Error(`Unknown option: ${arg}`)
  }
  if (!Number.isInteger(attempts) || attempts < 1) throw new Error('--attempts must be a positive integer')
  if ((casePath === null) === (suitePath === null)) throw new Error('Pass exactly one of --case or --suite')

  const startedAt = new Date().toISOString()
  const caseFiles: string[] = []
  let suiteName = 'single-case'
  let passThreshold = 1
  if (casePath) {
    caseFiles.push(resolve(casePath))
  } else {
    const suite = parseEvalSuite(JSON.parse(await readFile(resolve(suitePath!), 'utf8')))
    suiteName = suite.name
    passThreshold = suite.passThreshold
    for (const relativeCase of suite.cases) caseFiles.push(join(dirname(resolve(suitePath!)), relativeCase))
  }

  const cases: AgentEvalCase[] = []
  for (const file of caseFiles) cases.push(await loadEvalCase(file))

  const gatedMissing = cases.filter((evalCase) => evalCase.gate && !process.env[evalCase.gate.env])
  if (gatedMissing.length > 0) {
    for (const evalCase of gatedMissing) {
      process.stderr.write(`case ${evalCase.id} is gated on ${evalCase.gate!.env}; set it to run\n`)
    }
    return 4
  }
  if (cases.some((evalCase) => evalCase.worker.provider === 'stub')) {
    throw new Error('stub-provider cases are test-injected; the CLI runs real-provider cases only')
  }

  const providers = createEvalProviders()
  const results: Array<{ evalCase: AgentEvalCase; result: EvalRunResult }> = []
  for (const evalCase of cases) {
    for (let attempt = 1; attempt <= attempts; attempt += 1) {
      process.stdout.write(`running ${evalCase.id} attempt ${attempt}/${attempts}\n`)
      const result = await runEvalCase({ evalCase, providers, attempt })
      results.push({ evalCase, result })
      await writeAttemptArtifacts(runRoot, evalCase, result)
      process.stdout.write(`  -> ${result.finalStatus} (score ${result.finalScore})\n`)
    }
  }

  const summary = buildSuiteSummary({
    runId: runRoot.split('/').pop() ?? runRoot,
    suiteName,
    passThreshold,
    startedAt,
    finishedAt: new Date().toISOString(),
    results,
  })
  await writeSuiteSummary(runRoot, summary)
  process.stdout.write(`\nsummary: ${join(runRoot, 'summary.md')}\n`)
  return summary.passedThreshold ? 0 : 2
}

async function reportCommand(args: string[]): Promise<number> {
  const runDir = args[0]
  if (!runDir) throw new Error('usage: agent-eval report <run-directory>')
  const summary = await readFile(join(resolve(runDir), 'summary.md'), 'utf8')
  process.stdout.write(summary)
  return 0
}

function createEvalProviders(): Provider[] {
  return [
    createClaudeCodeProvider(),
    createCodexProvider({}),
    createOpenAIApiProvider(),
    createAnthropicApiProvider(),
  ]
}

function requiredValue(args: string[], index: number, flag: string): string {
  const value = args[index]
  if (!value) throw new Error(`${flag} requires a value`)
  return value
}

function printUsage(): void {
  process.stdout.write(`Usage:
  bun run agent-eval run --case <case.json> [--attempts N] [--out dir]
  bun run agent-eval run --suite <suite.json> [--attempts N] [--out dir]
  bun run agent-eval report <run-directory>
`)
}

if (import.meta.main) {
  runCli(process.argv.slice(2)).then(
    (code) => process.exit(code),
    (error) => {
      process.stderr.write(`fatal: ${errorMessage(error)}\n`)
      process.exit(1)
    },
  )
}
