import { asRecord, asString, sliceHead } from '@demicodes/utils'
import type {
  BashAuditEvent,
  ShellEnvironment,
  ShellAbortInput,
  ShellCommandStatus,
  ShellExecInput,
  ShellOutputChunk,
  ShellStatusInput,
  ShellWriteInput,
  ShellStreamView,
} from '@demicodes/shell'
import { bytesToBase64 } from '@demicodes/utils'
import { modelAcceptsMediaType, sniffModelMediaType, type Model, type ModelMediaKind, type ToolResultContentBlock } from '@demicodes/core'
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

const execRepeatStates = new WeakMap<ShellEnvironment, Map<string, ShellExecRepeatState>>()

export interface StandardAgentToolOptions<State = unknown> {
  environment:
    | ShellEnvironment
    | ((
        ctx: AgentToolInvokeContext<State>,
        handle: { shellId?: string; commandId?: string },
      ) => ShellEnvironment | Promise<ShellEnvironment>)
  scheduleYield(ctx: AgentToolInvokeContext<State>, durationMs: number): AgentToolInvokeResult
}

export function createStandardAgentTools<State = unknown>(
  options: StandardAgentToolOptions<State>,
): AgentTool<State>[] {
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
        const environment = await resolveEnvironment(options.environment, ctx, { shellId: parsed.shellId })
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
        const parsed = parseShellStatusInput(input)
        const environment = await resolveEnvironment(options.environment, ctx, { commandId: parsed.commandId })
        const result = await environment.status(parsed)
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
        const parsed = parseShellWriteInput(input)
        const environment = await resolveEnvironment(options.environment, ctx, { commandId: parsed.commandId })
        const result = await environment.write({ ...parsed, signal: ctx.signal })
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
        const parsed = parseShellAbortInput(input)
        const environment = await resolveEnvironment(options.environment, ctx, { commandId: parsed.commandId })
        const result = await environment.abort(parsed)
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

function resolveEnvironment<State>(
  source: StandardAgentToolOptions<State>['environment'],
  ctx: AgentToolInvokeContext<State>,
  handle: { shellId?: string; commandId?: string },
): ShellEnvironment | Promise<ShellEnvironment> {
  return typeof source === 'function' ? source(ctx, handle) : source
}

export interface ShellToolResultOptions {
  includePreview?: boolean
  previewBudgetTokens?: number
  exposeCommandHandle?: boolean
  /** Model receiving this result; gates binary-stream media attachment. */
  model?: Model
  /** Per-modality byte caps on attached media; unset modalities keep the defaults. */
  maxMediaBytes?: Partial<Record<ModelMediaKind, number>>
}

/**
 * How many bytes of each modality a tool may hand to the model.
 *
 * One number cannot serve both, because bytes buy wildly different amounts of
 * context per modality — measured against a frontier model, a KiB of video
 * costs ~2 tokens where a KiB of image costs ~50. A cap generous enough to show
 * a five-minute clip would let a single still eat a six-figure token budget.
 *
 * image (4 MiB): well past any sane still — a 4000x3000 PNG lands under it — so
 *   crossing this line is a mistake, not a use case.
 * video (16 MiB): roughly ten minutes at a viewing-grade encoding, and
 *   deliberately under the ~20 MB inline-payload ceiling the major APIs enforce.
 *   A larger cap buys no reach, only a rejection further downstream where the
 *   reason is harder to read.
 *
 * Bytes are a proxy, not a budget: for video they track cost reasonably at a
 * fixed encoding, but an image's real driver is its pixel dimensions.
 */
export const DEFAULT_MAX_MEDIA_BYTES: Record<ModelMediaKind, number> = {
  image: 4 * 1024 * 1024,
  video: 16 * 1024 * 1024,
}

export function shellPreviewBudgetTokens(contextWindow: number): number {
  return contextWindow >= LARGE_CONTEXT_THRESHOLD_TOKENS ? LARGE_CONTEXT_PREVIEW_TOKENS : SMALL_CONTEXT_PREVIEW_TOKENS
}

export function toShellToolResult(
  result: ShellCommandStatus,
  options: ShellToolResultOptions = {},
): AgentToolInvokeResult {
  const output: ToolResultContentBlock[] = [{ type: 'text', text: formatShellToolResult(result, options) }]
  if (result.status === 'exited' && result.binaryStdout) {
    const verdict = binaryStreamVerdict(result.binaryStdout, `${result.artifactDir}/stdout.bin`, options.model, {
      ...DEFAULT_MAX_MEDIA_BYTES,
      ...options.maxMediaBytes,
    })
    if (verdict.block) output.push(verdict.block)
    if (verdict.note) output.push({ type: 'text', text: verdict.note })
  }
  return {
    output,
    isError: false,
    view: shellToolView(result),
  }
}

/** Character budget for a shell view's render window (tail-biased). */
export const SHELL_VIEW_MAX_CHARS = 32_768

/**
 * Bounded UI view of a shell command stored on the tool_call block. Full
 * output lives in the command artifact directory (real files on the host
 * filesystem, see `artifactDir`), keyed by
 * `commandId`; the view carries only the tail render window and never embeds
 * raw or base64 bytes.
 */
export interface ShellToolView {
  kind: 'shell'
  status: 'running' | 'exited' | 'aborted'
  shellId: string
  commandId: string
  exitCode?: number
  runningMs: number
  idleMs: number
  /** Tail of the merged output, capped at SHELL_VIEW_MAX_CHARS. */
  chunks: ShellOutputChunk[]
  /** True when chunks were capped; the artifact has the full output. */
  viewTruncated: boolean
  audit?: BashAuditEvent[]
}

function shellToolView(result: ShellCommandStatus): ShellToolView {
  const window = tailChunkWindow(result.output.chunks, SHELL_VIEW_MAX_CHARS)
  const view: ShellToolView = {
    kind: 'shell',
    status: result.status,
    shellId: result.shellId,
    commandId: result.commandId,
    runningMs: result.runningMs,
    idleMs: result.idleMs,
    chunks: window.chunks,
    viewTruncated: window.truncated || result.output.truncated,
  }
  if (result.status === 'exited') {
    view.exitCode = result.exitCode
    if (result.audit.length > 0) view.audit = result.audit
  }
  return view
}

function tailChunkWindow(
  chunks: ShellOutputChunk[],
  maxChars: number,
): { chunks: ShellOutputChunk[]; truncated: boolean } {
  const kept: ShellOutputChunk[] = []
  let total = 0
  for (let i = chunks.length - 1; i >= 0; i -= 1) {
    const chunk = chunks[i]!
    if (chunk.text.length === 0) continue
    const remaining = maxChars - total
    if (remaining <= 0) return { chunks: kept, truncated: true }
    if (chunk.text.length <= remaining) {
      kept.unshift({ stream: chunk.stream, text: chunk.text })
      total += chunk.text.length
    } else {
      kept.unshift({ stream: chunk.stream, text: chunk.text.slice(chunk.text.length - remaining) })
      return { chunks: kept, truncated: true }
    }
  }
  return { chunks: kept, truncated: false }
}

/**
 * Boundary decision for a binary final stream: attach as native media when the
 * magic matches the closed model-media set, the model accepts the type, and
 * the stream was not truncated; otherwise explain why nothing was attached.
 */
function binaryStreamVerdict(
  binary: NonNullable<Extract<ShellCommandStatus, { status: 'exited' }>['binaryStdout']>,
  binPath: string,
  model: Model | undefined,
  maxMediaBytes: Record<ModelMediaKind, number>,
): { block?: ToolResultContentBlock; note?: string } {
  const media = sniffModelMediaType(binary.data)
  if (binary.truncated) {
    return {
      note: `Binary stdout (${binary.totalBytes} bytes${
        media ? `, ${media.mediaType}` : ''
      }) exceeded the shell's ${binary.limitBytes}-byte binary limit (maxBinaryBytes) and was not attached; the raw bytes remain readable at ${binPath}. Produce a smaller version and re-run.`,
    }
  }
  if (!media) {
    return {
      note: `Binary stdout does not match any model-viewable media type; the raw bytes remain readable at ${binPath}.`,
    }
  }
  if (!model || !modelAcceptsMediaType(model, media.mediaType)) {
    return {
      note: `Binary stdout is ${media.mediaType}, which this model does not accept natively; the raw bytes remain readable at ${binPath}.`,
    }
  }
  // Per-modality cap. This is the layer that knows both what the bytes are and
  // which model is about to receive them, so it is the layer that decides
  // whether they are worth the context — the shell below only bounds raw size.
  const cap = maxMediaBytes[media.kind]
  if (binary.totalBytes > cap) {
    return {
      note: `Binary stdout is ${media.mediaType} (${binary.totalBytes} bytes), over the ${cap}-byte ${media.kind} cap; it was not attached and the raw bytes remain readable at ${binPath}. Produce a smaller version — for video, fewer frames or a lower resolution — and re-run.`,
    }
  }
  const source = { mediaType: media.mediaType, data: bytesToBase64(binary.data) }
  return {
    block: media.kind === 'video' ? { type: 'video', source } : { type: 'image', source },
    note: `Attached stdout as ${media.mediaType} (${binary.totalBytes} bytes).`,
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
  environment: ShellEnvironment,
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
    view: {
      kind: 'repeated_shell_exec',
      script,
      count,
    },
  }
}

function formatShellToolResult(result: ShellCommandStatus, options: ShellToolResultOptions): string {
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
    lines.push(`metaPath: ${result.artifactDir}/meta.json`)
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

function appendArtifact(lines: string[], label: string, artifact: ShellStreamView): void {
  lines.push(`${label}Path: ${artifact.path}`)
  lines.push(`${label}Bytes: ${artifact.bytes}`)
}

function appendPreview(lines: string[], result: ShellCommandStatus, budgetTokens: number): void {
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
      `previewTruncated: true; read ${result.artifactDir}/stdout.txt or ${result.artifactDir}/stderr.txt for more.`,
    )
  }
}

function boundedPreview(text: string, budgetTokens: number): { text: string; truncated: boolean } {
  const maxChars = Math.max(0, Math.floor(budgetTokens * APPROX_CHARS_PER_TOKEN))
  if (maxChars === 0) return { text: '', truncated: text.length > 0 }
  if (text.length <= maxChars) return { text, truncated: false }
  return { text: sliceHead(text, maxChars), truncated: true }
}

export async function finishShellToolResult<State>(
  environment: ShellEnvironment,
  result: ShellCommandStatus,
  ctx: AgentToolInvokeContext<State>,
): Promise<AgentToolInvokeResult> {
  const previewBudgetTokens = shellPreviewBudgetTokens(ctx.model.model.contextWindow)
  const exposeCommandHandle = shellCommandHandleRequired(result, previewBudgetTokens)
  const toolResult = toShellToolResult(result, {
    includePreview: true,
    previewBudgetTokens,
    exposeCommandHandle,
    model: ctx.model.model,
  })
  if (!exposeCommandHandle) await environment.releaseCommand(result.commandId)
  return toolResult
}

export function shellCommandHandleRequired(result: ShellCommandStatus, budgetTokens: number): boolean {
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
