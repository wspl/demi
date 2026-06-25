import type {
  BashEnvironment,
  ShellAbortInput,
  ShellCommandSnapshot,
  ShellExecInput,
  ShellStatusInput,
  ShellWriteInput,
  StreamArtifact,
} from '@demi/shell'
import type { AgentTool, AgentToolInvokeContext, AgentToolInvokeResult } from './types'

const MAX_CONSECUTIVE_IDENTICAL_EXEC = 6
const REPEAT_WINDOW_MS = 60_000
const MAX_DELAY_MS = 600_000

interface ShellExecRepeatState {
  script: string
  count: number
  updatedAt: number
}

const execRepeatStates = new WeakMap<BashEnvironment, Map<string, ShellExecRepeatState>>()

export interface StandardAgentToolOptions<State = unknown> {
  environment: BashEnvironment
  scheduleYield(ctx: AgentToolInvokeContext<State>, durationMs: number): AgentToolInvokeResult
}

export function createStandardAgentTools<State = unknown>(
  options: StandardAgentToolOptions<State>,
): AgentTool<State>[] {
  const { environment } = options
  return [
    {
      name: 'shell_exec',
      description:
        'Start a shell script. yieldAfterMs observes briefly and never kills. If running, use yield then shell_status.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['script', 'yieldAfterMs'],
        properties: {
          script: { type: 'string' },
          description: {
            type: 'string',
            description: 'Concise user-visible intent title; no object-only labels, steps, tool names, ids, internal labels, or reasons.',
          },
          shellId: { type: 'string' },
          yieldAfterMs: { type: 'number', minimum: 1, maximum: MAX_DELAY_MS },
          maxOutputBytes: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const parsed = parseShellExecInput(input)
        const repeatGuard = repeatedShellExecResult(environment, ctx.agentSessionId, parsed.script)
        if (repeatGuard) return repeatGuard
        const result = await environment.exec({
          ...parsed,
          agentSessionId: ctx.agentSessionId,
          signal: ctx.signal,
        })
        ctx.emitProgress(result)
        return toShellToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_status',
      description:
        'Read command status and bounded stdout/stderr deltas by commandId. Does not wait or mutate.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['commandId'],
        properties: {
          commandId: { type: 'string' },
          description: {
            type: 'string',
            description: 'Concise user-visible intent title; no object-only labels, steps, tool names, ids, internal labels, or reasons.',
          },
          stdoutOffset: { type: 'number' },
          stderrOffset: { type: 'number' },
          maxOutputBytes: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.status(parseShellStatusInput(input))
        ctx.emitProgress(result)
        return toShellToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_write',
      description:
        'Write non-empty stdin to a running foreground command. Include a newline for line-oriented prompts.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['commandId', 'stdin'],
        properties: {
          commandId: { type: 'string' },
          description: {
            type: 'string',
            description: 'Concise user-visible intent title; no object-only labels, steps, tool names, ids, internal labels, or reasons.',
          },
          stdin: { type: 'string' },
          maxOutputBytes: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.write({ ...parseShellWriteInput(input), signal: ctx.signal })
        ctx.emitProgress(result)
        return toShellToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_abort',
      description:
        'Stop a running foreground command by commandId.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['commandId'],
        properties: {
          commandId: { type: 'string' },
          description: {
            type: 'string',
            description: 'Concise user-visible intent title; no object-only labels, steps, tool names, ids, internal labels, or reasons.',
          },
          maxOutputBytes: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.abort(parseShellAbortInput(input))
        ctx.emitProgress(result)
        return { ...toShellToolResult(result, ctx.toolCallId), isError: false }
      },
    },
    {
      name: 'yield',
      description:
        'End this turn and schedule a one-shot wakeup. Does not touch shell commands.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['durationMs'],
        properties: {
          description: {
            type: 'string',
            description: 'Concise user-visible intent title; no object-only labels, steps, tool names, ids, internal labels, or reasons.',
          },
          durationMs: { type: 'number', minimum: 1, maximum: MAX_DELAY_MS },
        },
      },
      invoke: (ctx, input) => options.scheduleYield(ctx, parseYieldDuration(input)),
    },
  ]
}

export function toShellToolResult(result: ShellCommandSnapshot, toolCallId = ''): AgentToolInvokeResult {
  return {
    output: [{ type: 'text', text: formatShellToolResult(result) }],
    isError: false,
    metadata: result,
    continuation:
      result.status === 'running'
        ? {
            toolCallId,
            shellId: result.shellId,
            commandId: result.commandId,
            status: 'running',
          }
        : undefined,
  }
}

function parseShellExecInput(input: unknown): ShellExecInput {
  const record = asRecord(input)
  if (typeof record.script !== 'string') throw new Error('shell_exec requires string field "script"')
  return {
    script: record.script,
    shellId: optionalString(record.shellId),
    yieldAfterMs: requiredDelay(record.yieldAfterMs, 'shell_exec field "yieldAfterMs"'),
    maxOutputBytes: optionalNumber(record.maxOutputBytes),
  }
}

function parseShellStatusInput(input: unknown): ShellStatusInput {
  const record = asRecord(input)
  if (typeof record.commandId !== 'string') throw new Error('shell_status requires string field "commandId"')
  return {
    commandId: record.commandId,
    stdoutOffset: optionalNumber(record.stdoutOffset),
    stderrOffset: optionalNumber(record.stderrOffset),
    maxOutputBytes: optionalNumber(record.maxOutputBytes),
  }
}

function parseShellWriteInput(input: unknown): ShellWriteInput {
  const record = asRecord(input)
  if (typeof record.commandId !== 'string') throw new Error('shell_write requires string field "commandId"')
  if (typeof record.stdin !== 'string') throw new Error('shell_write requires string field "stdin"')
  if (record.stdin.length === 0) throw new Error('shell_write field "stdin" must not be empty; use shell_status to poll')
  return {
    commandId: record.commandId,
    stdin: record.stdin,
    maxOutputBytes: optionalNumber(record.maxOutputBytes),
  }
}

function parseShellAbortInput(input: unknown): ShellAbortInput {
  const record = asRecord(input)
  if (typeof record.commandId !== 'string') throw new Error('shell_abort requires string field "commandId"')
  return { commandId: record.commandId, maxOutputBytes: optionalNumber(record.maxOutputBytes) }
}

function parseYieldDuration(input: unknown): number {
  const record = asRecord(input)
  return requiredDelay(record.durationMs, 'yield field "durationMs"')
}

function requiredDelay(value: unknown, label: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 1 || value > MAX_DELAY_MS) {
    throw new Error(`${label} must be between 1 and ${MAX_DELAY_MS}`)
  }
  return Math.floor(value)
}

function repeatedShellExecResult(
  environment: BashEnvironment,
  agentSessionId: string,
  script: string,
): AgentToolInvokeResult | null {
  const now = Date.now()
  const states = execRepeatStates.get(environment) ?? new Map<string, ShellExecRepeatState>()
  execRepeatStates.set(environment, states)

  const previous = states.get(agentSessionId)
  const withinWindow = previous && now - previous.updatedAt <= REPEAT_WINDOW_MS
  const count = previous && withinWindow && previous.script === script ? previous.count + 1 : 1
  states.set(agentSessionId, { script, count, updatedAt: now })

  if (count <= MAX_CONSECUTIVE_IDENTICAL_EXEC) return null

  return {
    output: [
      {
        type: 'text',
        text: [
          'Repeated identical shell_exec suppressed.',
          `The same script has been run ${count} consecutive times in this agent session.`,
          'Inspect the previous output, use a different command, or provide the final answer instead of repeating it.',
        ].join('\n'),
      },
    ],
    isError: true,
    metadata: {
      kind: 'repeated_identical_shell_exec',
      script,
      count,
    },
  }
}

function formatShellToolResult(result: ShellCommandSnapshot): string {
  const lines = [
    `status: ${result.status}`,
    `shellId: ${result.shellId}`,
    `commandId: ${result.commandId}`,
    `runningMs: ${result.runningMs}`,
    `idleMs: ${result.idleMs}`,
  ]

  if (result.status === 'exited') lines.push(`exitCode: ${result.exitCode}`)

  appendArtifact(lines, 'stdout', result.stdout)
  appendArtifact(lines, 'stderr', result.stderr)

  if (result.status === 'running') {
    lines.push('next: command is still running; use yield to pause this turn, then shell_status with commandId.')
  } else if (result.status === 'aborted') {
    lines.push('next: command was intentionally stopped.')
  }

  return lines.join('\n')
}

function appendArtifact(lines: string[], label: string, artifact: StreamArtifact): void {
  lines.push(`${label}:`)
  lines.push(artifact.delta || '(empty)')
  lines.push(`${label}Path: ${artifact.path}`)
  lines.push(`${label}Offset: ${artifact.offset}`)
  lines.push(`${label}Bytes: ${artifact.bytes}`)
  if (artifact.truncated) lines.push(`${label}: truncated`)
}

function asRecord(input: unknown): Record<string, unknown> {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error('agent tool input must be an object')
  }
  return input as Record<string, unknown>
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' ? value : undefined
}
