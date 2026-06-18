import type { AgentTool, AgentToolInvokeResult } from '@demi/base-agent'
import type {
  BashEnvironment,
  ShellAbortInput,
  ShellExecInput,
  ShellStdinInput,
  ShellToolResult,
  ShellWaitInput,
} from './environment'

export function createShellSessionTools<State = unknown>(environment: BashEnvironment): AgentTool<State>[] {
  return [
    {
      name: 'shell_exec',
      description: 'Execute a command in the long-lived shell session.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['script'],
        properties: {
          script: { type: 'string' },
          sessionId: { type: 'string' },
          yieldAfterMs: { type: 'number' },
          timeoutMs: { type: 'number' },
          outputLimitBytes: { type: 'number' },
          needsInputAfterMs: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.exec({ ...parseShellExecInput(input), signal: ctx.signal })
        ctx.emitProgress(result)
        return toToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_wait',
      description: 'Wait for the foreground command in a shell session until it exits or yields again.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['sessionId'],
        properties: {
          sessionId: { type: 'string' },
          yieldAfterMs: { type: 'number' },
          timeoutMs: { type: 'number' },
          outputLimitBytes: { type: 'number' },
          needsInputAfterMs: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.wait({ ...parseShellWaitInput(input), signal: ctx.signal })
        ctx.emitProgress(result)
        return toToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_input',
      description: 'Write stdin to the foreground command in a shell session.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['sessionId', 'stdin'],
        properties: {
          sessionId: { type: 'string' },
          stdin: { type: 'string' },
          yieldAfterMs: { type: 'number' },
          outputLimitBytes: { type: 'number' },
          needsInputAfterMs: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.input({ ...parseShellInput(input), signal: ctx.signal })
        ctx.emitProgress(result)
        return toToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_abort',
      description: 'Abort the foreground command in a shell session.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['sessionId'],
        properties: {
          sessionId: { type: 'string' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.abort(parseShellAbortInput(input))
        ctx.emitProgress(result)
        return toToolResult(result, ctx.toolCallId)
      },
    },
  ]
}

export function toToolResult(result: ShellToolResult, toolCallId = ''): AgentToolInvokeResult {
  return {
    output: [{ type: 'text', text: stringifyToolResult(result) }],
    isError: result.status === 'timeout' || result.status === 'aborted',
    metadata: result,
    continuation:
      result.status === 'running' || result.status === 'needs_input'
        ? {
            toolCallId,
            sessionId: result.sessionId,
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
    sessionId: optionalString(record.sessionId),
    yieldAfterMs: optionalNumber(record.yieldAfterMs),
    timeoutMs: optionalNumber(record.timeoutMs),
    outputLimitBytes: optionalNumber(record.outputLimitBytes),
    needsInputAfterMs: optionalNumber(record.needsInputAfterMs),
  }
}

function parseShellWaitInput(input: unknown): ShellWaitInput {
  const record = asRecord(input)
  if (typeof record.sessionId !== 'string') throw new Error('shell_wait requires string field "sessionId"')
  return {
    sessionId: record.sessionId,
    yieldAfterMs: optionalNumber(record.yieldAfterMs),
    timeoutMs: optionalNumber(record.timeoutMs),
    outputLimitBytes: optionalNumber(record.outputLimitBytes),
    needsInputAfterMs: optionalNumber(record.needsInputAfterMs),
  }
}

function parseShellInput(input: unknown): ShellStdinInput {
  const record = asRecord(input)
  if (typeof record.sessionId !== 'string') throw new Error('shell_input requires string field "sessionId"')
  if (typeof record.stdin !== 'string') throw new Error('shell_input requires string field "stdin"')
  return {
    sessionId: record.sessionId,
    stdin: record.stdin,
    yieldAfterMs: optionalNumber(record.yieldAfterMs),
    outputLimitBytes: optionalNumber(record.outputLimitBytes),
    needsInputAfterMs: optionalNumber(record.needsInputAfterMs),
  }
}

function parseShellAbortInput(input: unknown): ShellAbortInput {
  const record = asRecord(input)
  if (typeof record.sessionId !== 'string') throw new Error('shell_abort requires string field "sessionId"')
  return { sessionId: record.sessionId }
}

function asRecord(input: unknown): Record<string, unknown> {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    throw new Error('shell tool input must be an object')
  }
  return input as Record<string, unknown>
}

function optionalString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined
}

function optionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' ? value : undefined
}

function stringifyToolResult(result: ShellToolResult): string {
  return JSON.stringify(result, (_key, value) => {
    if (typeof value === 'bigint') return value.toString()
    return value
  })
}
