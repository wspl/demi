import { asRecord, asString } from '@demicodes/utils'
import type {
  BashEnvironment,
  ShellAbortInput,
  ShellCommandSnapshot,
  ShellExecInput,
  ShellStatusInput,
  ShellWriteInput,
  StreamArtifact,
} from '@demicodes/shell'
import type { ToolResultContentBlock } from '@demicodes/core'
import type { AgentTool, AgentToolInvokeContext, AgentToolInvokeResult } from './types'

const MAX_CONSECUTIVE_IDENTICAL_EXEC = 6
const REPEAT_WINDOW_MS = 60_000
const MAX_DELAY_MS = 600_000
const SMALL_CONTEXT_PREVIEW_TOKENS = 1_000
const LARGE_CONTEXT_PREVIEW_TOKENS = 10_000
const LARGE_CONTEXT_THRESHOLD_TOKENS = 800_000
const APPROX_CHARS_PER_TOKEN = 4
const TOOL_DESCRIPTION_FIELD =
  'Concise title for the concrete user-visible state or result to make visible or confirm. Do not describe waiting, pausing, tool mechanics, generic actions, object labels, steps, tool names, ids, internals, or reasons.'

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
        'Start a shell script and observe it for up to timeoutMs. timeoutMs is an observation window, not a kill deadline: at timeoutMs the command keeps running and a command handle (commandId) is returned. Completed short output is returned directly. shell_exec never ends the turn or schedules a wakeup on its own.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['script', 'timeoutMs'],
        properties: {
          script: { type: 'string' },
          description: {
            type: 'string',
            description: TOOL_DESCRIPTION_FIELD,
          },
          shellId: { type: 'string' },
          timeoutMs: { type: 'number', minimum: 1, maximum: MAX_DELAY_MS },
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
        return finishShellToolResult(environment, result, ctx)
      },
    },
    {
      name: 'shell_status',
      description:
        'Read a running command handle status and any new budgeted output preview. Does not wait or write stdin.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['commandId'],
        properties: {
          commandId: { type: 'string' },
          description: {
            type: 'string',
            description: TOOL_DESCRIPTION_FIELD,
          },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.status(parseShellStatusInput(input))
        ctx.emitProgress(result)
        return finishShellToolResult(environment, result, ctx)
      },
    },
    {
      name: 'shell_write',
      description:
        'Write non-empty stdin to a running foreground command and return status with new budgeted output preview. Include a newline for line-oriented prompts.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['commandId', 'stdin'],
        properties: {
          commandId: { type: 'string' },
          description: {
            type: 'string',
            description: TOOL_DESCRIPTION_FIELD,
          },
          stdin: { type: 'string' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.write({ ...parseShellWriteInput(input), signal: ctx.signal })
        ctx.emitProgress(result)
        return finishShellToolResult(environment, result, ctx)
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
            description: TOOL_DESCRIPTION_FIELD,
          },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.abort(parseShellAbortInput(input))
        ctx.emitProgress(result)
        return { ...(await finishShellToolResult(environment, result, ctx)), isError: false }
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
            description: TOOL_DESCRIPTION_FIELD,
          },
          durationMs: { type: 'number', minimum: 1, maximum: MAX_DELAY_MS },
        },
      },
      invoke: (ctx, input) => options.scheduleYield(ctx, parseYieldDuration(input)),
    },
  ]
}

export interface ShellToolResultOptions {
  includePreview?: boolean
  previewBudgetTokens?: number
  exposeCommandHandle?: boolean
}

export function shellPreviewBudgetTokens(contextWindow: number): number {
  return contextWindow >= LARGE_CONTEXT_THRESHOLD_TOKENS ? LARGE_CONTEXT_PREVIEW_TOKENS : SMALL_CONTEXT_PREVIEW_TOKENS
}

export function toShellToolResult(
  result: ShellCommandSnapshot,
  toolCallId = '',
  options: ShellToolResultOptions = {},
): AgentToolInvokeResult {
  const output: ToolResultContentBlock[] = [{ type: 'text', text: formatShellToolResult(result, options) }]
  if (result.status === 'exited' && result.assets) {
    for (const asset of result.assets) {
      output.push({ type: 'image', source: { mediaType: asset.mediaType, data: asset.data } })
    }
  }
  return {
    output,
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
  const record = asRecord(input, 'agent tool input must be an object')
  if (typeof record.script !== 'string') throw new Error('shell_exec requires string field "script"')
  return {
    script: record.script,
    shellId: asString(record.shellId),
    timeoutMs: requiredDelay(record.timeoutMs, 'shell_exec field "timeoutMs"'),
  }
}

function parseShellStatusInput(input: unknown): ShellStatusInput {
  const record = asRecord(input, 'agent tool input must be an object')
  if (typeof record.commandId !== 'string') throw new Error('shell_status requires string field "commandId"')
  return {
    commandId: record.commandId,
  }
}

function parseShellWriteInput(input: unknown): ShellWriteInput {
  const record = asRecord(input, 'agent tool input must be an object')
  if (typeof record.commandId !== 'string') throw new Error('shell_write requires string field "commandId"')
  if (typeof record.stdin !== 'string') throw new Error('shell_write requires string field "stdin"')
  if (record.stdin.length === 0) throw new Error('shell_write field "stdin" must not be empty; use shell_status to poll')
  return {
    commandId: record.commandId,
    stdin: record.stdin,
  }
}

function parseShellAbortInput(input: unknown): ShellAbortInput {
  const record = asRecord(input, 'agent tool input must be an object')
  if (typeof record.commandId !== 'string') throw new Error('shell_abort requires string field "commandId"')
  return { commandId: record.commandId }
}

function parseYieldDuration(input: unknown): number {
  const record = asRecord(input, 'agent tool input must be an object')
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

function formatShellToolResult(result: ShellCommandSnapshot, options: ShellToolResultOptions): string {
  const exposeCommandHandle = options.exposeCommandHandle ?? true
  const lines = [`status: ${result.status}`]

  if (result.status === 'exited') lines.push(`exitCode: ${result.exitCode}`)

  if (exposeCommandHandle) {
    lines.push(`shellId: ${result.shellId}`)
    lines.push(`commandId: ${result.commandId}`)
    lines.push(`runningMs: ${result.runningMs}`)
    lines.push(`idleMs: ${result.idleMs}`)
    appendArtifact(lines, 'stdout', result.stdout)
    appendArtifact(lines, 'stderr', result.stderr)
    lines.push(`metaPath: /@/commands/${result.commandId}/meta.json`)
  }

  if (options.includePreview) {
    appendPreview(lines, result, options.previewBudgetTokens ?? SMALL_CONTEXT_PREVIEW_TOKENS)
  }

  if (result.status === 'running') {
    lines.push(
      'next: command is still running; check again with shell_status, or call yield to end this turn and be woken later, or shell_abort to stop it.',
    )
  } else if (result.status === 'aborted') {
    lines.push('next: command was intentionally stopped.')
  } else if (exposeCommandHandle) {
    lines.push('next: command is complete; read the artifact only if the preview is insufficient.')
  }

  return lines.join('\n')
}

function appendArtifact(lines: string[], label: string, artifact: StreamArtifact): void {
  lines.push(`${label}Path: ${artifact.path}`)
  lines.push(`${label}Bytes: ${artifact.bytes}`)
}

function appendPreview(lines: string[], result: ShellCommandSnapshot, budgetTokens: number): void {
  const preview = boundedPreview(result.output.text, budgetTokens)
  lines.push(`previewBudgetTokens: ${budgetTokens}`)
  if (preview.text.length === 0) {
    lines.push('preview: (empty)')
    return
  }
  lines.push('preview:')
  lines.push(preview.text)
  if (preview.truncated || result.output.truncated) {
    lines.push(
      `previewTruncated: true; read /@/commands/${result.commandId}/stdout.txt or /@/commands/${result.commandId}/stderr.txt for more.`,
    )
  }
}

function boundedPreview(text: string, budgetTokens: number): { text: string; truncated: boolean } {
  const maxChars = Math.max(0, Math.floor(budgetTokens * APPROX_CHARS_PER_TOKEN))
  if (maxChars === 0) return { text: '', truncated: text.length > 0 }
  if (text.length <= maxChars) return { text, truncated: false }
  return { text: text.slice(0, maxChars), truncated: true }
}

async function finishShellToolResult<State>(
  environment: BashEnvironment,
  result: ShellCommandSnapshot,
  ctx: AgentToolInvokeContext<State>,
): Promise<AgentToolInvokeResult> {
  const previewBudgetTokens = shellPreviewBudgetTokens(ctx.model.model.contextWindow)
  const exposeCommandHandle = shellCommandHandleRequired(result, previewBudgetTokens)
  const toolResult = toShellToolResult(result, ctx.toolCallId, {
    includePreview: true,
    previewBudgetTokens,
    exposeCommandHandle,
  })
  if (!exposeCommandHandle) await environment.releaseCommand(result.commandId)
  return toolResult
}

export function shellCommandHandleRequired(result: ShellCommandSnapshot, budgetTokens: number): boolean {
  if (result.status === 'running') return true
  const preview = boundedPreview(result.output.text, budgetTokens)
  const maxChars = Math.max(0, Math.floor(budgetTokens * APPROX_CHARS_PER_TOKEN))
  return (
    preview.truncated ||
    result.output.truncated ||
    result.output.bytes > maxChars ||
    result.stdout.truncated ||
    result.stderr.truncated
  )
}


