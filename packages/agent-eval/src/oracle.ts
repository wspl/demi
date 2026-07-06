import { spawn } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import type { Block } from '@demicodes/core'
import type { OracleSpec, TranscriptAssertion } from './case-schema'
import { diffWorkspace, matchesPathPattern, type WorkspaceSnapshot } from './workspace'

/**
 * Oracles are the evidence source: objective checks the Evaluator judges from.
 * They never score — they only report what was observed.
 */

export interface OracleResult {
  name: string
  type: OracleSpec['type']
  passed: boolean
  summary: string
  detail: {
    stdout?: string
    stderr?: string
    exitCode?: number | null
    failures?: string[]
  }
}

export interface OracleContext {
  workspace: string
  transcriptBlocks: Block[]
  workspaceBefore: WorkspaceSnapshot
  workspaceAfter: WorkspaceSnapshot
}

export async function runOracles(specs: readonly OracleSpec[], context: OracleContext): Promise<OracleResult[]> {
  const results: OracleResult[] = []
  for (const spec of specs) {
    results.push(await runOracle(spec, context))
  }
  return results
}

async function runOracle(spec: OracleSpec, context: OracleContext): Promise<OracleResult> {
  switch (spec.type) {
    case 'command':
      return runCommandOracle(spec, context)
    case 'file':
      return runFileOracle(spec, context)
    case 'transcript':
      return runTranscriptOracle(spec, context)
    case 'diff':
      return runDiffOracle(spec, context)
  }
}

async function runCommandOracle(
  spec: Extract<OracleSpec, { type: 'command' }>,
  context: OracleContext,
): Promise<OracleResult> {
  const { stdout, stderr, exitCode } = await runProcess(spec.command, resolve(context.workspace, spec.cwd ?? '.'), spec.timeoutMs)
  const failures: string[] = []
  if (exitCode !== spec.expectedExitCode) failures.push(`exit code ${exitCode ?? 'signal'} (expected ${spec.expectedExitCode})`)
  for (const needle of spec.stdoutIncludes ?? []) {
    if (!stdout.includes(needle)) failures.push(`stdout missing "${needle}"`)
  }
  for (const needle of spec.stderrExcludes ?? []) {
    if (stderr.includes(needle)) failures.push(`stderr contains "${needle}"`)
  }
  return {
    name: spec.name,
    type: 'command',
    passed: failures.length === 0,
    summary: failures.length === 0 ? `${spec.command.join(' ')} ok` : failures.join('; '),
    detail: { stdout, stderr, exitCode, failures },
  }
}

async function runFileOracle(
  spec: Extract<OracleSpec, { type: 'file' }>,
  context: OracleContext,
): Promise<OracleResult> {
  const failures: string[] = []
  let content: string | null = null
  try {
    content = await readFile(resolve(context.workspace, spec.path), 'utf8')
  } catch {
    content = null
  }
  if (spec.mustExist && content === null) failures.push(`file ${spec.path} does not exist`)
  if (!spec.mustExist && content !== null) failures.push(`file ${spec.path} exists but must not`)
  if (content !== null) {
    for (const needle of spec.textIncludes ?? []) {
      if (!content.includes(needle)) failures.push(`file ${spec.path} missing "${needle}"`)
    }
  }
  return {
    name: `file:${spec.path}`,
    type: 'file',
    passed: failures.length === 0,
    summary: failures.length === 0 ? `${spec.path} ok` : failures.join('; '),
    detail: { failures },
  }
}

function runTranscriptOracle(
  spec: Extract<OracleSpec, { type: 'transcript' }>,
  context: OracleContext,
): OracleResult {
  const failures: string[] = []
  for (const assertion of spec.assertions) {
    const failure = checkTranscriptAssertion(assertion, context.transcriptBlocks)
    if (failure) failures.push(failure)
  }
  return {
    name: 'transcript',
    type: 'transcript',
    passed: failures.length === 0,
    summary: failures.length === 0 ? 'transcript assertions ok' : failures.join('; '),
    detail: { failures },
  }
}

function checkTranscriptAssertion(assertion: TranscriptAssertion, blocks: Block[]): string | null {
  switch (assertion.kind) {
    case 'tool_call': {
      const count = blocks.filter(
        (block) => block.type === 'tool_call' && (!assertion.toolName || block.toolName === assertion.toolName),
      ).length
      return count >= assertion.minCount
        ? null
        : `expected >=${assertion.minCount} ${assertion.toolName ?? 'tool'} calls, saw ${count}`
    }
    case 'steer': {
      const count = blocks.filter((block) => block.type === 'steer' && !block.hidden).length
      return count >= assertion.minCount ? null : `expected >=${assertion.minCount} steers, saw ${count}`
    }
    case 'abort': {
      const count = blocks.filter((block) => block.type === 'abort').length
      return count >= assertion.minCount ? null : `expected >=${assertion.minCount} aborts, saw ${count}`
    }
    case 'compaction': {
      const count = blocks.filter((block) => block.type === 'compaction_boundary').length
      return count >= assertion.minCount ? null : `expected >=${assertion.minCount} compactions, saw ${count}`
    }
    case 'assistant_text_includes': {
      const found = blocks.some((block) => block.type === 'text' && block.text.includes(assertion.text))
      return found ? null : `assistant text missing "${assertion.text}"`
    }
  }
}

function runDiffOracle(spec: Extract<OracleSpec, { type: 'diff' }>, context: OracleContext): OracleResult {
  const entries = diffWorkspace(context.workspaceBefore, context.workspaceAfter)
  const failures: string[] = []
  for (const entry of entries) {
    if (spec.forbiddenPaths?.some((pattern) => matchesPathPattern(entry.path, pattern))) {
      failures.push(`forbidden path touched: ${entry.path}`)
      continue
    }
    if (spec.allowedPaths && spec.allowedPaths.length > 0) {
      if (!spec.allowedPaths.some((pattern) => matchesPathPattern(entry.path, pattern))) {
        failures.push(`path outside allowed set: ${entry.path} (${entry.status})`)
      }
    }
  }
  return {
    name: 'diff',
    type: 'diff',
    passed: failures.length === 0,
    summary: failures.length === 0 ? `workspace diff ok (${entries.length} entries)` : failures.join('; '),
    detail: { failures },
  }
}

function runProcess(
  command: readonly string[],
  cwd: string,
  timeoutMs: number,
): Promise<{ stdout: string; stderr: string; exitCode: number | null }> {
  return new Promise((resolvePromise) => {
    const [executable, ...args] = command
    const child = spawn(executable!, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    let settled = false
    const timer = setTimeout(() => {
      if (settled) return
      settled = true
      child.kill('SIGKILL')
      resolvePromise({ stdout, stderr: `${stderr}\n(oracle timed out after ${timeoutMs}ms)`, exitCode: null })
    }, timeoutMs)
    child.stdout.on('data', (chunk: Buffer) => {
      stdout += chunk.toString('utf8')
    })
    child.stderr.on('data', (chunk: Buffer) => {
      stderr += chunk.toString('utf8')
    })
    child.on('error', (error) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      resolvePromise({ stdout, stderr: `${stderr}\n${error.message}`, exitCode: null })
    })
    child.on('close', (code) => {
      if (settled) return
      settled = true
      clearTimeout(timer)
      resolvePromise({ stdout, stderr, exitCode: code })
    })
  })
}
