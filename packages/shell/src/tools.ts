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
      description:
        'Execute a command in a long-lived shell session. Returns exited or running with a shellId for continuation. Run observable long-lived commands in the foreground with yieldAfterMs, then use shell_wait or shell_abort instead of backgrounding and pkill/killall.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['script'],
        properties: {
          script: { type: 'string' },
          shellId: { type: 'string' },
          yieldAfterMs: { type: 'number' },
          timeoutMs: { type: 'number' },
          outputLimitBytes: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.exec({
          ...parseShellExecInput(input),
          agentSessionId: ctx.agentSessionId,
          signal: ctx.signal,
        })
        ctx.emitProgress(result)
        return toToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_wait',
      description:
        'Poll or wait for the foreground command in a shell session. Each call waits from the current call time, not from process start.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['shellId'],
        properties: {
          shellId: { type: 'string' },
          yieldAfterMs: { type: 'number' },
          timeoutMs: { type: 'number' },
          outputLimitBytes: { type: 'number' },
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
      description:
        'Write explicit stdin to the current foreground system process in a shell session. Use shell_wait to poll. For interactive stdin, keep the reader inside one foreground process such as sh -c, node, or python; do not rely on the session script builtin read across turns.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['shellId', 'stdin'],
        properties: {
          shellId: { type: 'string' },
          stdin: { type: 'string' },
          yieldAfterMs: { type: 'number' },
          outputLimitBytes: { type: 'number' },
        },
      },
      invoke: async (ctx, input) => {
        const parsed = parseShellInput(input)
        const result = await environment.input({ ...parsed, signal: ctx.signal })
        ctx.emitProgress(result)
        return toToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_abort',
      description: 'Stop the foreground command in a shell session. This is an intentional control action.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['shellId'],
        properties: {
          shellId: { type: 'string' },
        },
      },
      invoke: async (ctx, input) => {
        const result = await environment.abort(parseShellAbortInput(input))
        ctx.emitProgress(result)
        return { ...toToolResult(result, ctx.toolCallId), isError: false }
      },
    },
  ]
}

export function toToolResult(result: ShellToolResult, toolCallId = ''): AgentToolInvokeResult {
  return {
    output: [{ type: 'text', text: formatToolResult(result) }],
    isError: result.status === 'timeout' || result.status === 'aborted',
    metadata: result,
    continuation:
      result.status === 'running'
        ? {
            toolCallId,
            shellId: result.shellId,
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
    yieldAfterMs: optionalNumber(record.yieldAfterMs),
    timeoutMs: optionalNumber(record.timeoutMs),
    outputLimitBytes: optionalNumber(record.outputLimitBytes),
  }
}

function parseShellWaitInput(input: unknown): ShellWaitInput {
  const record = asRecord(input)
  if (typeof record.shellId !== 'string') throw new Error('shell_wait requires string field "shellId"')
  return {
    shellId: record.shellId,
    yieldAfterMs: optionalNumber(record.yieldAfterMs),
    timeoutMs: optionalNumber(record.timeoutMs),
    outputLimitBytes: optionalNumber(record.outputLimitBytes),
  }
}

function parseShellInput(input: unknown): ShellStdinInput {
  const record = asRecord(input)
  if (typeof record.shellId !== 'string') throw new Error('shell_input requires string field "shellId"')
  if (typeof record.stdin !== 'string') throw new Error('shell_input requires string field "stdin"')
  if (record.stdin.length === 0) throw new Error('shell_input field "stdin" must not be empty; use shell_wait to poll')
  return {
    shellId: record.shellId,
    stdin: record.stdin,
    yieldAfterMs: optionalNumber(record.yieldAfterMs),
    outputLimitBytes: optionalNumber(record.outputLimitBytes),
  }
}

function parseShellAbortInput(input: unknown): ShellAbortInput {
  const record = asRecord(input)
  if (typeof record.shellId !== 'string') throw new Error('shell_abort requires string field "shellId"')
  return { shellId: record.shellId }
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

function formatToolResult(result: ShellToolResult): string {
  const lines = [`status: ${result.status}`, `shellId: ${result.shellId}`]

  if (result.status === 'exited') {
    lines.push(`exitCode: ${result.exitCode}`)
  } else {
    lines.push(`runningMs: ${result.runningMs}`)
  }

  if (result.status === 'running') {
    lines.push(`reason: ${result.reason}`, `idleMs: ${result.idleMs}`)
  }

  appendOutput(lines, result.output)

  if (result.status === 'running') {
    lines.push('next: command is still running; call shell_wait to poll, shell_input with stdin to interact, or shell_abort to stop it intentionally. For dev servers or watchers, prefer shell_abort over process-name kills.')
  } else if (result.status === 'timeout') {
    lines.push('next: command exceeded timeoutMs and was stopped.')
  } else if (result.status === 'aborted') {
    lines.push('next: command was intentionally stopped.')
  }

  return lines.join('\n')
}

function appendOutput(lines: string[], output: ShellToolResult['output']): void {
  lines.push('stdout:')
  lines.push(output.stdoutDelta || '(empty)')
  lines.push('stderr:')
  lines.push(output.stderrDelta || '(empty)')
  if (output.truncated) lines.push('output: truncated')
}
