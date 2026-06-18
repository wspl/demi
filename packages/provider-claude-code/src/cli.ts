import { spawn } from 'node:child_process'
import { readdir, readFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import type { ProviderAuthState, ProviderRuntimeState } from '@demi/provider'

export interface ClaudeCliStatus {
  path: string | null
  version: string | null
}

export interface ShellCommandResult {
  stdout: string
  stderr: string
  exitCode: number | null
}

export type ShellRunner = (command: string, args: string[]) => Promise<ShellCommandResult>

export interface ClaudeStatsigState {
  exists: boolean
  stableId?: string
  sessionId?: string
}

export type StatsigReader = () => Promise<ClaudeStatsigState>

export async function detectClaudeCli(run: ShellRunner = runShellCommand): Promise<ClaudeCliStatus> {
  const which = await run('which', ['claude'])
  if (which.exitCode !== 0) return { path: null, version: null }

  const path = which.stdout.trim()
  const version = await run(path, ['--version'])
  return {
    path,
    version: version.exitCode === 0 ? version.stdout.trim() : null,
  }
}

export async function claudeRuntimeState(run: ShellRunner = runShellCommand): Promise<ProviderRuntimeState> {
  const status = await detectClaudeCli(run)
  if (!status.path) return { status: 'unavailable', message: 'claude CLI is not installed or not on PATH' }
  return { status: 'ready', message: status.version ?? status.path }
}

export async function claudeAuthState(
  run: ShellRunner = runShellCommand,
  readStatsig: StatsigReader = readClaudeStatsigState,
): Promise<ProviderAuthState> {
  const auth = await run('claude', ['auth', 'status', '--json'])
  if (auth.exitCode !== 0) {
    const fallback = await readStatsig()
    const message = auth.stderr.trim() || 'Unable to query auth status'
    if (fallback.exists) {
      return { status: 'unknown', message: `${message}; Claude statsig state exists` }
    }
    return { status: 'unknown', message }
  }
  try {
    const json = JSON.parse(auth.stdout) as { loggedIn?: boolean; email?: string; authMethod?: string }
    if (json.loggedIn) return { status: 'authenticated', accountLabel: json.email ?? json.authMethod }
    return { status: 'unauthenticated' }
  } catch (error) {
    return { status: 'error', message: error instanceof Error ? error.message : String(error) }
  }
}

export async function readClaudeStatsigState(
  statsigDir = join(homedir(), '.claude', 'statsig'),
): Promise<ClaudeStatsigState> {
  try {
    const entries = await readdir(statsigDir)
    const stableFile = entries.find((entry) => entry.startsWith('statsig.stable_id.'))
    const sessionFile = entries.find((entry) => entry.startsWith('statsig.session_id.'))
    return {
      exists: Boolean(stableFile || sessionFile),
      stableId: stableFile ? await readTrimmed(join(statsigDir, stableFile)) : undefined,
      sessionId: sessionFile ? await readTrimmed(join(statsigDir, sessionFile)) : undefined,
    }
  } catch {
    return { exists: false }
  }
}

export function buildClaudeArgs(params: {
  modelId: string
  systemPrompt: string
  thinkingEffort?: string | null
  maxBudgetUsd?: number | string | null
}): string[] {
  const args = [
    '--print',
    '--output-format',
    'stream-json',
    '--verbose',
    '--input-format',
    'stream-json',
    '--no-session-persistence',
    '--safe-mode',
    '--disable-slash-commands',
    '--tools',
    '',
    '--model',
    params.modelId,
    '--system-prompt',
    params.systemPrompt,
  ]
  if (params.thinkingEffort) args.push('--effort', params.thinkingEffort)
  if (params.maxBudgetUsd !== undefined && params.maxBudgetUsd !== null) {
    args.push('--max-budget-usd', String(params.maxBudgetUsd))
  }
  return args
}

export function buildClaudeEnv(base: NodeJS.ProcessEnv = process.env): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...base,
    DISABLE_AUTO_COMPACT: '1',
    MAX_MCP_OUTPUT_TOKENS: '1000000',
  }
  delete env.CLAUDECODE
  return env
}

export async function runShellCommand(command: string, args: string[]): Promise<ShellCommandResult> {
  const child = spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'] })
  const [stdout, stderr, exit] = await Promise.all([
    collect(child.stdout),
    collect(child.stderr),
    new Promise<{ exitCode: number | null }>((resolve) => {
      child.once('close', (exitCode) => resolve({ exitCode }))
      child.once('error', () => resolve({ exitCode: null }))
    }),
  ])
  return { stdout, stderr, exitCode: exit.exitCode }
}

async function collect(stream: NodeJS.ReadableStream): Promise<string> {
  const chunks: Uint8Array[] = []
  for await (const chunk of stream) chunks.push(chunk instanceof Uint8Array ? chunk : Buffer.from(String(chunk)))
  return Buffer.concat(chunks).toString('utf8')
}

async function readTrimmed(path: string): Promise<string | undefined> {
  try {
    const text = await readFile(path, 'utf8')
    return text.trim() || undefined
  } catch {
    return undefined
  }
}
