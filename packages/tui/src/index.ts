import { mkdir } from 'node:fs/promises'
import process from 'node:process'
import { createInterface } from 'node:readline/promises'
import { resolve } from 'node:path'
import type {
  Block,
  FileExtension,
  ModelSelection,
  SessionPhase,
  ThinkingCapability,
  ThinkingEffort,
  ThinkingSummary,
  TokenUsage,
  UserContentBlock,
} from '@demi/core'
import { createCodingAgentHarness } from '@demi/coding-agent'
import { ProviderRegistry, type ProviderModel, type ProviderModelList } from '@demi/provider'
import { createClaudeCodeProviderDefinition } from '@demi/provider-claude-code'
import { FileCodexAuthStore, createCodexProviderDefinition, type CodexTransportMode } from '@demi/provider-codex'
import {
  AgentClient,
  AgentServer,
  type ClientSessionEvent,
  type ProviderConfig,
} from '@demi/agent'
import { LocalHost } from '@demi/shell/local-host'

export interface TuiOptions {
  provider: 'claude-code' | 'codex'
  cwd: string
  modelId: string | null
  thinkingEffort: ThinkingEffort | null
  maxBudgetUsd: string | null
  claudePath?: string
  codexHome?: string
  baseUrl?: string
  transport: CodexTransportMode
  yieldAfterMs: number
  timeoutMs: number
}

export interface TuiOutput {
  write(text: string): void
  isTTY?: boolean
}

export interface RenderState {
  output: TuiOutput
  phase: SessionPhase | null
  textLengths: Map<string, number>
  thinkingLengths: Map<string, number>
  seenThinkingSignatures: Set<string>
  toolStatuses: Map<string, string>
  seenResponseIds: Set<string>
  seenUserIds: Set<string>
  seenErrorIds: Set<string>
  seenAbortIds: Set<string>
  toolOutputCounts: Map<string, number>
  activeStream: 'assistant' | 'thinking' | null
  streamAtLineStart: boolean
}

interface TuiCommandClient {
  abort(): Promise<boolean>
  retry(): Promise<void>
  resume(): Promise<void>
  compact(): Promise<void>
  shellInput(shellId: string, stdin: string): Promise<void>
}

interface TuiLoopClient extends TuiCommandClient {
  send(content: UserContentBlock[]): Promise<void>
}

interface TuiEventSource {
  subscribe(listener: (event: ClientSessionEvent) => void): () => void
}

export interface TuiInputLoop {
  ask(): Promise<string>
  client: TuiLoopClient
  renderer: RenderState
  output?: TuiOutput
}

const helpText = `Commands:
  /help                 Show this help
  /abort                Abort the active turn
  /retry                Retry the latest user turn
  /resume               Resume after an abort
  /compact              Request transcript compaction
  /input <shellId> <text>
                       Send stdin to a foreground shell session
  /exit                 Close the session

Tips:
  Start in a scratch directory for acceptance tests.
  Messages are sent asynchronously, so /abort can be typed while a turn is running.
  Example prompt: "Create src/app.ts, add a todo to run tests, then run cat src/app.ts."`

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2))
  await mkdir(options.cwd, { recursive: true })

  const host = new LocalHost(options.cwd)
  const harness = createCodingAgentHarness({ host })

  const providerRegistry = new ProviderRegistry()
  providerRegistry.register(createClaudeCodeProviderDefinition())
  providerRegistry.register(createCodexProviderDefinition())
  const providerConfigData = providerConfigForOptions(options)
  const model = await resolveTuiModel(providerRegistry, options, providerConfigData)

  printBanner(options, model)
  if (options.provider === 'codex') await printCodexAuthStatus(options)

  const server = new AgentServer({
    agent: harness,
    providerRegistry,
    shell: {
      initialEnv: { PATH: process.env.PATH ?? '' },
      yieldAfterMs: options.yieldAfterMs,
      timeoutMs: options.timeoutMs,
    },
  })
  const client = server.client()
  const renderer = createRenderer()
  attachRenderer(client, renderer)

  const providerConfig: ProviderConfig = {
    type: options.provider,
    config: providerConfigData,
    model: model.selection,
  }

  await client.open(providerConfig, options.cwd)
  writeLine(color('Session opened. Type /help for commands, /exit to quit.', 'dim'))

  const rl = createInterface({ input: process.stdin, output: process.stdout })
  let closing = false
  process.on('SIGINT', () => {
    if (closing) return
    if (renderer.phase && renderer.phase !== 'idle') {
      writeLine(`\n${color('Aborting active turn...', 'yellow')}`)
      void client.abort().catch((error) => writeLine(color(`abort failed: ${messageOf(error)}`, 'red')))
      return
    }
    closing = true
    writeLine(`\n${color('Closing...', 'dim')}`)
    void cleanup(rl, client, server).finally(() => process.exit(0))
  })

  try {
    await runInputLoop({
      ask: () => rl.question(color('\nyou> ', 'bold')),
      client,
      renderer,
      output: process.stdout,
      shouldContinue: () => !closing,
    })
  } finally {
    await cleanup(rl, client, server)
  }
}

export async function runInputLoop(options: TuiInputLoop & { shouldContinue?: () => boolean }): Promise<void> {
  const output = options.output ?? process.stdout
  while (options.shouldContinue?.() ?? true) {
    const input = (await options.ask()).trim()
    if (!input) continue
    if (input.startsWith('/')) {
      const shouldExit = await handleCommand(input, options.client, output)
      if (shouldExit) break
      continue
    }

    const content: UserContentBlock[] = [{ type: 'text', text: input }]
    void options.client.send(content).catch((error) => {
      finishStream(options.renderer)
      writeLineTo(output, color(`send failed: ${messageOf(error)}`, 'red', output))
    })
  }
}

export async function handleCommand(
  input: string,
  client: TuiCommandClient,
  output: TuiOutput = process.stdout,
): Promise<boolean> {
  const [command, ...rest] = input.split(/\s+/)
  switch (command) {
    case '/help':
      writeLineTo(output, helpText)
      return false
    case '/abort':
      writeLineTo(output, color('abort requested', 'yellow', output))
      void client.abort().catch((error) => writeLineTo(output, color(`abort failed: ${messageOf(error)}`, 'red', output)))
      return false
    case '/retry':
      void client.retry().catch((error) => writeLineTo(output, color(`retry failed: ${messageOf(error)}`, 'red', output)))
      return false
    case '/resume':
      void client.resume().catch((error) => writeLineTo(output, color(`resume failed: ${messageOf(error)}`, 'red', output)))
      return false
    case '/compact':
      void client.compact().catch((error) => writeLineTo(output, color(`compact failed: ${messageOf(error)}`, 'red', output)))
      return false
    case '/input': {
      const shellId = rest.shift()
      if (!shellId) {
        writeLineTo(output, color('usage: /input <shellId> <text>', 'red', output))
        return false
      }
      void client
        .shellInput(shellId, `${rest.join(' ')}\n`)
        .catch((error) => writeLineTo(output, color(`input failed: ${messageOf(error)}`, 'red', output)))
      return false
    }
    case '/exit':
    case '/quit':
      return true
    default:
      writeLineTo(output, color(`Unknown command: ${command}`, 'red', output))
      return false
  }
}

async function cleanup(
  rl: ReturnType<typeof createInterface>,
  client: AgentClient,
  server: AgentServer,
): Promise<void> {
  rl.close()
  try {
    await client.close()
  } finally {
    await server.close()
  }
}

export function renderEvent(state: RenderState, event: ClientSessionEvent): void {
  switch (event.type) {
    case 'opened':
      return
    case 'phase':
      if (event.phase !== state.phase) {
        finishStream(state)
        state.phase = event.phase
        writeLineTo(state.output, color(`status: ${event.phase}`, event.phase === 'idle' ? 'green' : 'yellow', state.output))
      }
      return
    case 'queue':
      finishStream(state)
      if (event.queue.length > 0) {
        writeLineTo(state.output, color(`queue: ${event.queue.length} message(s)`, 'dim', state.output))
      }
      return
    case 'shell_output':
      finishStream(state)
      renderShellOutput(state, event.shellId, event.snapshot.stdoutDelta, event.snapshot.stderrDelta)
      return
    case 'audit':
      finishStream(state)
      for (const item of event.events) {
        if (item.kind === 'registered-command') {
          writeLineTo(
            state.output,
            color(`audit: registered ${item.name} ${item.args.join(' ')} -> ${item.exitCode}`, 'dim', state.output),
          )
        } else {
          writeLineTo(
            state.output,
            color(`audit: system ${item.name} ${item.args.join(' ')} -> ${item.exitCode ?? 'signal'}`, 'dim', state.output),
          )
        }
      }
      return
    case 'tool_progress':
      renderToolProgress(state, event.output)
      return
    case 'shell_input_result':
      finishStream(state)
      return
    case 'error':
      finishStream(state)
      writeLineTo(state.output, color(`error: ${event.message}`, 'red', state.output))
      return
    case 'rejected':
      finishStream(state)
      writeLineTo(state.output, color(`rejected ${event.command}: ${event.reason}`, 'red', state.output))
      return
    case 'closed':
      finishStream(state)
      writeLineTo(state.output, color('closed', 'dim', state.output))
      return
    case 'transcript_snapshot':
      renderBlocks(state, event.blocks)
      return
    case 'transcript_patch':
      renderBlocks(state, event.blocks)
      return
  }
}

export function attachRenderer(source: TuiEventSource, state: RenderState): () => void {
  return source.subscribe((event) => renderEvent(state, event))
}

function renderBlocks(state: RenderState, blocks: Block[]): void {
  for (const block of blocks) {
    if (block.type === 'user') {
      if (!state.seenUserIds.has(block.id)) state.seenUserIds.add(block.id)
      continue
    }
    if (block.type === 'text') {
      const previous = state.textLengths.get(block.id) ?? 0
      const delta = block.text.slice(previous)
      if (delta) writeStreamDelta(state, 'assistant', 'assistant> ', 'blue', delta)
      state.textLengths.set(block.id, block.text.length)
      continue
    }
    if (block.type === 'thinking') {
      const previous = state.thinkingLengths.get(block.id) ?? 0
      const delta = block.text.slice(previous)
      if (delta) writeStreamDelta(state, 'thinking', 'thinking> ', 'dim', delta)
      state.thinkingLengths.set(block.id, block.text.length)
      if (block.signature && !state.seenThinkingSignatures.has(block.id)) {
        state.seenThinkingSignatures.add(block.id)
        if (block.text.length === 0) {
          finishStream(state)
          writeLineTo(state.output, color('thinking> [signed]', 'dim', state.output))
        }
      }
      continue
    }
    if (block.type === 'redacted_thinking') {
      if (!state.thinkingLengths.has(block.id)) {
        finishStream(state)
        writeLineTo(state.output, color(`thinking> [redacted ${block.data.length} chars]`, 'dim', state.output))
        state.thinkingLengths.set(block.id, block.data.length)
      }
      continue
    }
    if (block.type === 'tool_call') {
      const marker = `${block.status}:${block.output.length}:${block.streamingOutput.length}`
      if (state.toolStatuses.get(block.id) !== marker) {
        finishStream(state)
        state.toolStatuses.set(block.id, marker)
        writeLineTo(state.output, color(`tool: ${block.toolName} ${block.status} ${formatToolInput(block)}`, 'cyan', state.output))
      }
      renderToolCallOutput(state, block)
      continue
    }
    if (block.type === 'response' && !state.seenResponseIds.has(block.id)) {
      finishStream(state)
      state.seenResponseIds.add(block.id)
      writeLineTo(state.output, color(`usage: ${formatUsage(block.usage)}`, 'dim', state.output))
      continue
    }
    if (block.type === 'error' && !state.seenErrorIds.has(block.id)) {
      finishStream(state)
      state.seenErrorIds.add(block.id)
      writeLineTo(state.output, color(`agent error: ${block.message}`, 'red', state.output))
      continue
    }
    if (block.type === 'abort' && !state.seenAbortIds.has(block.id)) {
      finishStream(state)
      state.seenAbortIds.add(block.id)
      writeLineTo(state.output, color('turn aborted', 'yellow', state.output))
      continue
    }
  }
}

function renderToolCallOutput(state: RenderState, block: Extract<Block, { type: 'tool_call' }>): void {
  if (block.status !== 'error') return
  const previous = state.toolOutputCounts.get(block.id) ?? 0
  const next = block.output.slice(previous)
  if (next.length === 0) return
  finishStream(state)
  state.toolOutputCounts.set(block.id, block.output.length)
  const text = next.map((item) => (item.type === 'text' ? item.text : `[image:${item.source.mediaType}]`)).join('\n')
  writePrefixed(state.output, 'tool error', text, 'red')
}

function renderShellOutput(state: RenderState, shellId: string, stdoutDelta: string, stderrDelta: string): void {
  if (stdoutDelta) writePrefixed(state.output, `shell[${shellId}] stdout`, stdoutDelta, 'green')
  if (stderrDelta) writePrefixed(state.output, `shell[${shellId}] stderr`, stderrDelta, 'red')
}

function renderToolProgress(state: RenderState, output: Extract<ClientSessionEvent, { type: 'tool_progress' }>['output']): void {
  const text = output.map((block) => (block.type === 'text' ? block.text : `[image:${block.source.mediaType}]`)).join('\n')
  const shell = parseShellProgress(text)
  if (shell) {
    finishStream(state)
    writeLineTo(
      state.output,
      color(`progress: shell[${shell.shellId}] ${shell.status}${shell.reason ? ` (${shell.reason})` : ''}`, 'dim', state.output),
    )
    return
  }
  if (text.trim()) {
    finishStream(state)
    writePrefixed(state.output, 'progress', text, 'dim')
  }
}

function parseShellProgress(text: string): { shellId: string; status: string; reason?: string } | null {
  try {
    const value = JSON.parse(text) as Record<string, unknown>
    if (typeof value.shellId !== 'string' || typeof value.status !== 'string') return null
    return {
      shellId: value.shellId,
      status: value.status,
      reason: typeof value.reason === 'string' ? value.reason : undefined,
    }
  } catch {
    return null
  }
}

function writeStreamDelta(
  state: RenderState,
  stream: 'assistant' | 'thinking',
  label: string,
  tone: Tone,
  delta: string,
): void {
  if (state.activeStream !== stream) {
    finishStream(state)
    state.output.write(`\n${color(label, tone, state.output)}`)
    state.activeStream = stream
    state.streamAtLineStart = false
  }
  state.output.write(stream === 'thinking' ? color(delta, 'dim', state.output) : delta)
  state.streamAtLineStart = delta.endsWith('\n')
}

function finishStream(state: RenderState): void {
  if (state.activeStream && !state.streamAtLineStart) state.output.write('\n')
  state.activeStream = null
  state.streamAtLineStart = true
}

function writePrefixed(output: TuiOutput, label: string, text: string, tone: Tone): void {
  const lines = text.endsWith('\n') ? text.slice(0, -1).split('\n') : text.split('\n')
  for (const line of lines) writeLineTo(output, `${color(`${label}>`, tone, output)} ${line}`)
}

function formatUsage(usage: TokenUsage): string {
  return [
    `in=${usage.inputTokens}`,
    `out=${usage.outputTokens}`,
    `cache_read=${usage.cacheReadTokens}`,
    `cache_write=${usage.cacheWriteTokens}`,
  ].join(' ')
}

function formatToolInput(block: Extract<Block, { type: 'tool_call' }>): string {
  try {
    const input = JSON.parse(block.input) as Record<string, unknown>
    if (block.toolName === 'shell_exec' && typeof input.script === 'string') {
      return trimOneLine(input.script)
    }
    if (block.toolName === 'shell_wait' && typeof input.shellId === 'string') return input.shellId
    if (block.toolName === 'shell_input' && typeof input.shellId === 'string') return input.shellId
    if (block.toolName === 'shell_abort' && typeof input.shellId === 'string') return input.shellId
  } catch {
    // Fall through to raw input.
  }
  return trimOneLine(block.input)
}

function trimOneLine(text: string): string {
  const compact = text.replace(/\s+/g, ' ').trim()
  return compact.length > 100 ? `${compact.slice(0, 97)}...` : compact
}

export function createRenderer(output: TuiOutput = process.stdout): RenderState {
  return {
    output,
    phase: null,
    textLengths: new Map(),
    thinkingLengths: new Map(),
    seenThinkingSignatures: new Set(),
    toolStatuses: new Map(),
    seenResponseIds: new Set(),
    seenUserIds: new Set(),
    seenErrorIds: new Set(),
    seenAbortIds: new Set(),
    toolOutputCounts: new Map(),
    activeStream: null,
    streamAtLineStart: true,
  }
}

function parseArgs(args: string[]): TuiOptions {
  let provider: TuiOptions['provider'] = parseProvider(process.env.DEMI_PROVIDER ?? 'claude-code')
  let cwd = process.cwd()
  let modelId: string | null = null
  let thinkingEffort = parseThinkingEffort(envThinkingValue(provider), envThinkingSource(provider))
  let thinkingProvided = false
  let maxBudgetUsd: string | null = process.env.DEMI_CLAUDE_CODE_MAX_BUDGET_USD ?? '0.25'
  let claudePath: string | undefined
  let codexHome: string | undefined = process.env.CODEX_HOME
  let baseUrl: string | undefined
  let transport: CodexTransportMode = 'auto'
  let yieldAfterMs = 10_000
  let timeoutMs = 120_000

  for (let index = 0; index < args.length; index++) {
    const arg = args[index]
    if (arg === '--help' || arg === '-h') {
      printUsage()
      process.exit(0)
    }
    if (arg === '--cwd') cwd = requiredValue(args, ++index, '--cwd')
    else if (arg === '--provider') provider = parseProvider(requiredValue(args, ++index, '--provider'))
    else if (arg === '--model') {
      modelId = requiredValue(args, ++index, '--model')
    }
    else if (arg === '--thinking') {
      thinkingEffort = parseThinkingEffort(requiredValue(args, ++index, '--thinking'), '--thinking')
      thinkingProvided = true
    } else if (arg === '--no-thinking') {
      thinkingEffort = null
      thinkingProvided = true
    }
    else if (arg === '--budget') maxBudgetUsd = requiredValue(args, ++index, '--budget')
    else if (arg === '--no-budget') maxBudgetUsd = null
    else if (arg === '--claude-path') claudePath = requiredValue(args, ++index, '--claude-path')
    else if (arg === '--codex-home') codexHome = requiredValue(args, ++index, '--codex-home')
    else if (arg === '--base-url') baseUrl = requiredValue(args, ++index, '--base-url')
    else if (arg === '--transport') transport = parseCodexTransport(requiredValue(args, ++index, '--transport'))
    else if (arg === '--yield-after-ms') yieldAfterMs = Number(requiredValue(args, ++index, '--yield-after-ms'))
    else if (arg === '--timeout-ms') timeoutMs = Number(requiredValue(args, ++index, '--timeout-ms'))
    else if (!arg.startsWith('-')) cwd = arg
    else throw new Error(`Unknown option: ${arg}`)
  }

  if (!thinkingProvided) thinkingEffort = parseThinkingEffort(envThinkingValue(provider), envThinkingSource(provider))

  return {
    provider,
    cwd: resolve(cwd),
    modelId,
    thinkingEffort,
    maxBudgetUsd,
    claudePath,
    codexHome,
    baseUrl,
    transport,
    yieldAfterMs,
    timeoutMs,
  }
}

function envThinkingValue(provider: TuiOptions['provider']): string | null {
  if (provider === 'codex') return process.env.DEMI_CODEX_THINKING ?? process.env.DEMI_CLAUDE_CODE_THINKING ?? null
  return process.env.DEMI_CLAUDE_CODE_THINKING ?? null
}

function envThinkingSource(provider: TuiOptions['provider']): string {
  if (provider === 'codex' && process.env.DEMI_CODEX_THINKING !== undefined) return 'DEMI_CODEX_THINKING'
  return 'DEMI_CLAUDE_CODE_THINKING'
}

function parseProvider(value: string): TuiOptions['provider'] {
  if (value === 'claude-code' || value === 'codex') return value
  throw new Error('--provider must be one of: claude-code, codex')
}

function parseCodexTransport(value: string): CodexTransportMode {
  if (value === 'auto' || value === 'sse' || value === 'websocket') return value
  throw new Error('--transport must be one of: auto, sse, websocket')
}

function requiredValue(args: string[], index: number, flag: string): string {
  const value = args[index]
  if (!value) throw new Error(`${flag} requires a value`)
  return value
}

function parseThinkingEffort(value: string | null, source: string): ThinkingEffort | null {
  if (value === null || value === '') return null
  if (value === 'low' || value === 'medium' || value === 'high' || value === 'xhigh' || value === 'max') return value
  throw new Error(`${source} must be one of: low, medium, high, xhigh, max`)
}

export interface ResolvedTuiModel {
  selection: ModelSelection
  warnings: string[]
  catalog: ProviderModelList | null
}

export async function resolveTuiModel(
  registry: ProviderRegistry,
  options: TuiOptions,
  providerConfig: Record<string, unknown>,
): Promise<ResolvedTuiModel> {
  if (options.modelId) {
    validateExplicitModelId(options.provider, options.modelId)
    return {
      selection: modelSelectionFromCatalogModel(options.provider, options.modelId, options.thinkingEffort, null),
      warnings: [],
      catalog: null,
    }
  }

  let catalog: ProviderModelList | null = null
  let catalogError: unknown = null
  try {
    catalog = await registry.listModels(options.provider, providerConfig)
  } catch (error) {
    catalogError = error
  }

  if (!catalog) {
    throw new Error(`Unable to load ${options.provider} model catalog: ${messageOf(catalogError)}`)
  }
  if (catalog.models.length === 0) {
    throw new Error(`${options.provider} model catalog returned no models`)
  }
  const selected =
    (catalog.defaultModelId ? catalog.models.find((model) => model.id === catalog.defaultModelId) : null) ?? catalog.models[0]
  if (!selected) throw new Error(`${options.provider} model catalog returned no selectable models`)
  validateThinkingEffortForCatalogModel(options.thinkingEffort, selected)
  return {
    selection: modelSelectionFromCatalogModel(options.provider, selected.id, options.thinkingEffort, selected),
    warnings: [...catalog.warnings],
    catalog,
  }
}

function validateExplicitModelId(provider: TuiOptions['provider'], modelId: string): void {
  if (modelId === 'opus' || modelId === 'sonnet' || modelId === 'haiku' || modelId === 'default') {
    throw new Error(`--model must be a full ${provider} model id, not alias "${modelId}"`)
  }
  if (provider === 'claude-code' && !modelId.startsWith('claude-')) {
    throw new Error('--model for claude-code must be a full Claude model id such as claude-opus-4-8')
  }
  if (provider === 'codex' && !modelId.startsWith('gpt-') && !modelId.startsWith('codex-')) {
    throw new Error('--model for codex must be a full Codex model id such as gpt-5.5')
  }
}

function validateThinkingEffortForCatalogModel(thinkingEffort: ThinkingEffort | null, model: ProviderModel): void {
  if (!thinkingEffort) return
  const supported = model.supportedThinkingEfforts
  if (!supported || supported.length === 0) {
    throw new Error(`Model ${model.id} does not advertise explicit thinking effort controls`)
  }
  if (!supported.includes(thinkingEffort)) {
    throw new Error(`Model ${model.id} does not support thinking effort "${thinkingEffort}"`)
  }
}

function modelSelectionFromCatalogModel(
  provider: TuiOptions['provider'],
  modelId: string,
  thinkingEffort: ThinkingEffort | null,
  model: ProviderModel | null,
): ModelSelection {
  return {
    providerId: provider,
    model: {
      id: modelId,
      name: model?.displayName ?? `${provider === 'codex' ? 'Codex' : 'Claude Code'} ${modelId}`,
      contextWindow: model?.contextWindow ?? 0,
      inputLimit: null,
      thinking: thinkingCapabilitiesFromProviderModel(model),
      acceptedExtensions: model?.supportsAttachments ? acceptedAttachmentExtensions() : [],
    },
    thinking: thinkingEffort ? { type: 'effort', effort: thinkingEffort, summary: null } : null,
  }
}

function thinkingCapabilitiesFromProviderModel(model: ProviderModel | null): ThinkingCapability[] {
  if (!model) return []
  if (model.supportsReasoning === false) return [{ type: 'disabled' as const }]
  if (!model.supportedThinkingEfforts || model.supportedThinkingEfforts.length === 0) return []
  const summaries: ThinkingSummary[] = ['auto', 'concise', 'detailed', 'off', 'on']
  return [
    {
      type: 'effort' as const,
      efforts: model.supportedThinkingEfforts,
      defaultEffort: null,
      summaries,
      defaultSummary: null,
    },
  ]
}

function acceptedAttachmentExtensions(): FileExtension[] {
  return ['png', 'jpg', 'jpeg', 'gif', 'webp', 'pdf']
}

async function printCodexAuthStatus(options: TuiOptions): Promise<void> {
  const auth = await new FileCodexAuthStore({ codexHome: options.codexHome }).status()
  writeLine(color(`codex auth: ${auth.status}${'accountLabel' in auth && auth.accountLabel ? ` (${auth.accountLabel})` : ''}${'message' in auth && auth.message ? ` (${auth.message})` : ''}`, auth.status === 'authenticated' ? 'green' : 'yellow'))
}

function providerConfigForOptions(options: TuiOptions): Record<string, unknown> {
  if (options.provider === 'claude-code') {
    return {
      ...(options.claudePath ? { claudePath: options.claudePath } : {}),
      ...(options.maxBudgetUsd === null ? {} : { maxBudgetUsd: options.maxBudgetUsd }),
    }
  }
  return {
    ...(options.codexHome ? { codexHome: options.codexHome } : {}),
    ...(options.baseUrl ? { baseUrl: options.baseUrl } : {}),
    transport: options.transport,
  }
}

function printBanner(options: TuiOptions, model: ResolvedTuiModel): void {
  writeLine(color('Demi TUI acceptance shell', 'bold'))
  writeLine(`provider: ${options.provider}`)
  writeLine(`workspace: ${options.cwd}`)
  writeLine(`model: ${model.selection.model.id}`)
  writeLine(`thinking: ${options.thinkingEffort ?? 'not requested'}`)
  for (const warning of model.warnings) writeLine(color(`model warning: ${warning}`, 'yellow'))
  if (options.provider === 'claude-code') writeLine(`budget: ${options.maxBudgetUsd ?? 'none'}`)
  else writeLine(`transport: ${options.transport}`)
}

function printUsage(): void {
  writeLine(`Usage: bun run tui -- [workspace] [options]

Options:
  --cwd <path>             Workspace root. Defaults to current directory.
  --provider <id>          Provider: claude-code, codex. Defaults to claude-code.
  --model <id>             Full model id. Defaults to the provider model catalog selection.
  --thinking <effort>      Thinking effort: low, medium, high, xhigh, max.
  --no-thinking            Do not request an explicit thinking effort. This is the default.
  --budget <usd>           Max budget passed to claude. Defaults to 0.25.
  --no-budget              Do not pass a max budget.
  --claude-path <path>     Path to claude CLI. Defaults to claude on PATH.
  --codex-home <path>      Codex home containing auth.json. Defaults to CODEX_HOME or ~/.codex.
  --base-url <url>         Override Codex/OpenAI base URL.
  --transport <mode>       Codex transport: auto, sse, websocket. Defaults to auto.
  --yield-after-ms <n>     Shell yield interval. Defaults to 10000.
  --timeout-ms <n>         Shell command timeout. Defaults to 120000.

${helpText}`)
}

function writeLine(text = ''): void {
  process.stdout.write(`${text}\n`)
}

function writeLineTo(output: TuiOutput, text = ''): void {
  output.write(`${text}\n`)
}

type Tone = 'red' | 'green' | 'yellow' | 'blue' | 'cyan' | 'dim' | 'bold'

function color(text: string, tone: Tone, output: TuiOutput = process.stdout): string {
  if (!output.isTTY) return text
  const codes: Record<Tone, [number, number]> = {
    red: [31, 39],
    green: [32, 39],
    yellow: [33, 39],
    blue: [34, 39],
    cyan: [36, 39],
    dim: [2, 22],
    bold: [1, 22],
  }
  const [open, close] = codes[tone]
  return `\x1b[${open}m${text}\x1b[${close}m`
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

if (import.meta.main) {
  main().catch((error) => {
    process.stderr.write(`fatal: ${messageOf(error)}\n`)
    process.exit(1)
  })
}
