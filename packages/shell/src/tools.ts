import type {
  BashEnvironment,
  ShellAbortInput,
  ShellExecInput,
  ShellStdinInput,
  ShellToolResult,
  ShellWaitInput,
} from './environment'

export interface ShellToolInvokeContext<State> {
  agentSessionId: string
  state: State
  cwd: string
  toolCallId: string
  signal: AbortSignal
  emitProgress(progress: unknown): void
}

export interface ShellToolInvokeResult {
  output: Array<{ type: 'text'; text: string }>
  isError?: boolean
  metadata?: unknown | null
  continuation?: {
    toolCallId: string
    shellId: string
    status: 'running'
  }
  stopAfterToolResult?: boolean
}

export interface ShellAgentTool<State = unknown> {
  name: string
  description: string
  inputSchema: Record<string, unknown>
  invoke(ctx: ShellToolInvokeContext<State>, input: unknown): Promise<ShellToolInvokeResult> | ShellToolInvokeResult
}

const MAX_CONSECUTIVE_IDENTICAL_EXEC = 6
const REPEAT_WINDOW_MS = 60_000

interface ShellExecRepeatState {
  script: string
  count: number
  updatedAt: number
}

const execRepeatStates = new WeakMap<BashEnvironment, Map<string, ShellExecRepeatState>>()

export function createShellSessionTools<State = unknown>(environment: BashEnvironment): ShellAgentTool<State>[] {
  return [
    {
      name: 'shell_exec',
      description:
        'Execute a command in a long-lived shell session. Returns exited or running with a shellId for continuation. Run observable long-lived commands in the foreground with yieldAfterMs, then use shell_wait or shell_abort instead of backgrounding and pkill/killall. If the default shell already has a foreground process and you omit shellId, shell_exec runs the new command in an auxiliary shell; keep using the original shellId to wait/input/abort the foreground process. Always set "description" to a short, clear summary of what the command does — it is shown as the command\'s title in the UI.',
      inputSchema: {
        type: 'object',
        additionalProperties: false,
        required: ['script'],
        properties: {
          script: { type: 'string' },
          description: {
            type: 'string',
            description:
              'A clear, concise description of what this command does, in 5-10 words (e.g. "List files in the current directory"). Shown as the command\'s title in the UI, with the script itself shown below it.',
          },
          shellId: { type: 'string' },
          yieldAfterMs: { type: 'number' },
          timeoutMs: { type: 'number' },
          outputLimitBytes: { type: 'number' },
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
        return toToolResult(result, ctx.toolCallId)
      },
    },
    {
      name: 'shell_wait',
      description:
        'Poll or wait for the foreground command in a shell session. Each call waits from the current call time, not from process start. Use yieldAfterMs for short status polls. timeoutMs is a hard stop: when it expires, the foreground process is stopped.',
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
        'Write explicit stdin bytes to the current foreground system process in a shell session. Include a newline, such as "Alice\\n", when answering line-oriented prompts. Use shell_wait to poll. For interactive stdin, keep the reader inside one foreground process such as sh -c, node, or python; do not rely on the session script builtin read across turns.',
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

export function toToolResult(result: ShellToolResult, toolCallId = ''): ShellToolInvokeResult {
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

function repeatedShellExecResult(
  environment: BashEnvironment,
  agentSessionId: string,
  script: string,
): ShellToolInvokeResult | null {
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
