import type {
  ModelSelection,
  QueuedMessage,
  SessionPhase,
  TokenUsage,
  UserContentBlock,
} from '@demi/core'
import type { AgentProvider, InferenceRequest, ProviderEvent, ToolDefinition } from '@demi/provider'
import { Transcript, type TranscriptOptions } from './transcript'
import type {
  AgentHarnessRuntime,
  AgentSessionOptions,
  AgentSessionParams,
  AgentSessionRestoreParams,
  AgentSessionStore,
  AgentTool,
  AgentToolInvokeResult,
  ExternalMutationReservation,
  SessionEvent,
  SessionEventListener,
} from './types'

const DEFAULT_KEEP_RECENT_TOKENS = 4_000
const DEFAULT_PREFLIGHT_THRESHOLD_RATIO = 0.8

type PendingAction =
  | {
      type: 'send'
      id: string
      content: UserContentBlock[]
      resolve: () => void
      reject: (error: unknown) => void
    }
  | { type: 'retry'; resolve: () => void; reject: (error: unknown) => void }
  | { type: 'resume'; resolve: () => void; reject: (error: unknown) => void }
  | { type: 'compact'; resolve: () => void; reject: (error: unknown) => void }
  | {
      type: 'set_model'
      provider: AgentProvider | null
      model: ModelSelection
      resolve: () => void
      reject: (error: unknown) => void
    }

export class AgentSession<State> {
  private provider: AgentProvider
  private model: ModelSelection
  private readonly cwd: string
  private readonly runtime: AgentHarnessRuntime<State>
  private readonly agentSessionId: string
  private readonly idFactory: () => string
  private readonly store?: AgentSessionStore<State>
  private readonly compactionKeepRecentTokens: number
  private readonly compactionThresholdRatio: number
  private readonly listeners = new Set<SessionEventListener>()
  private readonly pendingActions: PendingAction[] = []
  private readonly queued: QueuedMessage[] = []

  private readonly transcriptLog: Transcript
  private readonly agentState: State
  private currentPhase: SessionPhase = 'idle'
  private workerRunning = false
  private externalMutationReserved = false
  private currentAbortController: AbortController | null = null
  private activeTurnId: string | null = null
  private abortRecorded = false
  private idleResolvers: Array<() => void> = []

  static fromSnapshot<State>(
    params: AgentSessionRestoreParams<State>,
    options: AgentSessionOptions<State> = {},
  ): AgentSession<State> {
    if (params.snapshot.harnessName !== params.runtime.harnessName) {
      throw new Error(
        `AgentSession: snapshot harness "${params.snapshot.harnessName}" does not match "${params.runtime.harnessName}"`,
      )
    }
    const snapshot = structuredClone(params.snapshot)
    return new AgentSession(
      {
        provider: params.provider,
        model: snapshot.model,
        cwd: snapshot.cwd,
        runtime: params.runtime,
        transcript: snapshot.transcript,
        state: snapshot.state,
      },
      options,
    )
  }

  constructor(params: AgentSessionParams<State>, options: AgentSessionOptions<State> = {}) {
    this.provider = params.provider
    this.model = params.model
    this.cwd = params.cwd
    this.runtime = params.runtime
    this.agentState = params.state === undefined ? params.runtime.initialState() : structuredClone(params.state)
    this.agentSessionId = options.agentSessionId ?? defaultIdFactory()
    this.idFactory = options.idFactory ?? defaultIdFactory
    this.store = options.store
    this.compactionKeepRecentTokens = options.compaction?.keepRecentTokens ?? DEFAULT_KEEP_RECENT_TOKENS
    this.compactionThresholdRatio =
      options.compaction?.preflightThresholdRatio ?? DEFAULT_PREFLIGHT_THRESHOLD_RATIO

    const transcriptOptions: TranscriptOptions = {
      idFactory: this.idFactory,
      now: options.now,
    }
    this.transcriptLog =
      params.transcript instanceof Transcript
        ? params.transcript
        : new Transcript(params.transcript?.blocks ?? [], transcriptOptions)
  }

  send(content: UserContentBlock[]): Promise<void> {
    const id = this.idFactory()
    return this.enqueue({
      type: 'send',
      id,
      content,
      resolve: noop,
      reject: noop,
    })
  }

  retry(): Promise<void> {
    return this.enqueue({ type: 'retry', resolve: noop, reject: noop })
  }

  resume(): Promise<void> {
    return this.enqueue({ type: 'resume', resolve: noop, reject: noop })
  }

  compact(): Promise<void> {
    return this.enqueue({ type: 'compact', resolve: noop, reject: noop })
  }

  /**
   * Switches the model (and optionally the provider) for the rest of the session. Queued as an
   * action so it lands at a turn boundary, never mid-tool-continuation; the next turn uses it.
   * A non-null `provider` replaces the current one (its predecessor is disposed); pass null to
   * keep the same provider instance and only change the model.
   */
  updateModel(provider: AgentProvider | null, model: ModelSelection): Promise<void> {
    return this.enqueue({ type: 'set_model', provider, model, resolve: noop, reject: noop })
  }

  async abort(): Promise<boolean> {
    if (!this.currentAbortController || this.currentAbortController.signal.aborted) return false

    this.currentAbortController.abort()
    await this.recordAbort()
    return true
  }

  waitUntilDone(): Promise<void> {
    if (!this.workerRunning && this.pendingActions.length === 0) return Promise.resolve()
    return new Promise((resolve) => {
      this.idleResolvers.push(resolve)
    })
  }

  /**
   * Tears the session down: aborts any in-flight turn and releases provider-held resources
   * (e.g. a long-lived CLI subprocess). Called when the owning connection closes.
   */
  async dispose(): Promise<void> {
    await this.abort()
    await this.provider.dispose?.()
  }

  transcript(): Transcript {
    return this.transcriptLog
  }

  state(): State {
    return this.agentState
  }

  id(): string {
    return this.agentSessionId
  }

  phase(): SessionPhase {
    return this.currentPhase
  }

  queuedMessages(): QueuedMessage[] {
    return this.queued.map((message) => ({ ...message, content: [...message.content] }))
  }

  subscribe(listener: SessionEventListener): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  reserveMutation(): ExternalMutationReservation {
    if (this.workerRunning || this.pendingActions.length > 0 || this.externalMutationReserved) {
      throw new Error('AgentSession: cannot reserve mutation while session is busy')
    }
    this.externalMutationReserved = true
    let released = false
    return {
      release: () => {
        if (released) return
        released = true
        this.externalMutationReserved = false
      },
    }
  }

  private enqueue(action: PendingAction): Promise<void> {
    if (this.externalMutationReserved) {
      return Promise.reject(new Error('AgentSession: cannot enqueue action while external mutation is reserved'))
    }

    return new Promise((resolve, reject) => {
      action.resolve = resolve
      action.reject = reject

      if (action.type === 'send' && (this.workerRunning || this.pendingActions.length > 0)) {
        this.queued.push({
          id: action.id,
          text: textContentSummary(action.content),
          content: action.content,
        })
        this.emitQueue()
      }

      this.pendingActions.push(action)
      this.kickWorker()
    })
  }

  private kickWorker(): void {
    if (this.workerRunning) return
    this.workerRunning = true
    void this.runWorker()
  }

  private async runWorker(): Promise<void> {
    try {
      while (this.pendingActions.length > 0) {
        const action = this.pendingActions.shift()
        if (!action) continue

        if (action.type === 'send') this.removeQueuedMessage(action.id)

        this.currentAbortController = new AbortController()
        this.activeTurnId = action.type === 'send' ? action.id : this.idFactory()
        this.abortRecorded = false

        try {
          await this.executeAction(action)
          action.resolve()
        } catch (error) {
          if (isAbortError(error)) {
            await this.recordAbort()
            action.resolve()
          } else {
            const normalized = asError(error)
            this.emit({ type: 'error', error: normalized })
            action.reject(normalized)
          }
        } finally {
          this.currentAbortController = null
          this.activeTurnId = null
          this.abortRecorded = false
          this.setPhase('idle')
        }
      }
    } finally {
      this.workerRunning = false
      this.resolveIdleWaiters()
    }
  }

  private async executeAction(action: PendingAction): Promise<void> {
    switch (action.type) {
      case 'send':
        this.setPhase('running')
        await this.executeSend(action.content)
        return
      case 'retry':
        this.setPhase('running')
        await this.executeRetry()
        return
      case 'resume':
        this.setPhase('running')
        await this.executeResume()
        return
      case 'compact':
        this.setPhase('compacting')
        await this.executeCompaction()
        return
      case 'set_model':
        await this.applyModelChange(action.provider, action.model)
        return
    }
  }

  private async applyModelChange(provider: AgentProvider | null, model: ModelSelection): Promise<void> {
    // Moving to a model with a smaller context window: compact with the CURRENT (pre-switch)
    // model + provider first — it can still load the whole history to summarize it — then hand
    // the now-smaller context to the new model. Doing it after the swap would ask the smaller
    // model to summarize a history it may not even be able to load.
    await this.compactToFitModel(model)
    if (provider && provider !== this.provider) {
      const previous = this.provider
      this.provider = provider
      await previous.dispose?.()
    }
    this.model = model
  }

  private async compactToFitModel(nextModel: ModelSelection): Promise<void> {
    const contextWindow = nextModel.model.contextWindow
    if (contextWindow <= 0) return
    const threshold = Math.floor(contextWindow * this.compactionThresholdRatio)
    if (this.transcriptLog.estimateContextTokens() < threshold) return

    const previousPhase = this.currentPhase
    this.setPhase('compacting')
    try {
      for (let attempt = 0; attempt < 8 && this.transcriptLog.estimateContextTokens() >= threshold; attempt += 1) {
        if (!(await this.executeCompaction())) break
      }
    } finally {
      this.setPhase(previousPhase)
    }
  }

  private async executeSend(content: UserContentBlock[]): Promise<void> {
    await this.runtime.lifecycle?.({
      type: 'before_round_start',
      agentSessionId: this.agentSessionId,
      state: this.agentState,
      transcript: this.transcriptLog,
      content,
    })

    const resolvedContent = await this.resolveReferences(content)
    const promptContext = this.promptContext()
    const preamble = this.runtime.preamble?.(promptContext) ?? null
    this.transcriptLog.pushUserTurn(this.model, resolvedContent, preamble)
    await this.commitTranscript()

    await this.executePreflightCompaction()
    await this.executeProviderTurn()
  }

  private async resolveReferences(content: UserContentBlock[]): Promise<UserContentBlock[]> {
    const resolver = this.runtime.resolveReferences
    if (!resolver) return content
    const signal = this.currentSignal()
    const resolved = await abortable(
      Promise.resolve(
        resolver(
          {
            state: this.agentState,
            agentSessionId: this.agentSessionId,
            cwd: this.cwd,
            transcript: this.transcriptLog,
            signal,
          },
          content,
        ),
      ),
      signal,
    )
    return resolved
  }

  private async executeRetry(): Promise<void> {
    const lastUserIndex = findLastUserTurnIndex(this.transcriptLog.blocks)
    if (lastUserIndex === null) throw new Error('AgentSession: cannot retry without a user turn')

    this.transcriptLog.blocks.splice(lastUserIndex + 1)
    await this.runtime.lifecycle?.({
      type: 'after_transcript_rewrite',
      agentSessionId: this.agentSessionId,
      state: this.agentState,
      transcript: this.transcriptLog,
      reason: 'retry',
    })
    await this.commitTranscript()

    await this.executePreflightCompaction()
    await this.executeProviderTurn()
  }

  private async executeResume(): Promise<void> {
    this.transcriptLog.markLatestAbortResumed()
    this.transcriptLog.pushResumeTurn(this.model)
    await this.commitTranscript()
    await this.executePreflightCompaction()
    await this.executeProviderTurn()
  }

  private async executePreflightCompaction(): Promise<void> {
    const contextWindow = this.model.model.contextWindow
    if (contextWindow <= 0) return
    const threshold = Math.floor(contextWindow * this.compactionThresholdRatio)
    if (this.transcriptLog.estimateContextTokens() < threshold) return

    const previousPhase = this.currentPhase
    this.setPhase('compacting')
    await this.executeCompaction()
    this.setPhase(previousPhase)
  }

  private async executeProviderTurn(): Promise<void> {
    while (true) {
      throwIfAborted(this.currentSignal())
      const shouldAutoRecover = await this.streamProviderOnce()
      throwIfAborted(this.currentSignal())

      const toolExecution = await this.executePendingTools()
      if (shouldAutoRecover) {
        const previousPhase = this.currentPhase
        this.setPhase('compacting')
        const compacted = await this.executeCompaction()
        this.setPhase(previousPhase)
        if (compacted) {
          this.transcriptLog.pushResumeTurn(this.model)
          await this.commitTranscript()
          continue
        }
      }
      if (toolExecution.stopAfterToolResult) return
      if (!toolExecution.executed) return
    }
  }

  private async streamProviderOnce(): Promise<boolean> {
    const request = this.buildInferenceRequest()
    let shouldAutoRecover = false
    for await (const event of this.providerEvents(request)) {
      throwIfAborted(request.cancel)
      if (event.type === 'abort') throw new AbortError()
      await this.applyProviderEvent(event)
      if (event.type === 'error') throw new ProviderStreamError(event.message, event.code)
      if (event.type === 'response' && this.isUsageNearLimit(event.usage)) {
        shouldAutoRecover = true
      }
    }
    return shouldAutoRecover
  }

  private async executePendingTools(): Promise<{ executed: boolean; stopAfterToolResult: boolean }> {
    const pending = this.transcriptLog.pendingToolCalls()
    if (pending.length === 0) return { executed: false, stopAfterToolResult: false }

    const tools = this.currentTools()
    const toolsByName = new Map(tools.map((tool) => [tool.name, tool]))
    let stopAfterToolResult = false

    for (const toolCall of pending) {
      throwIfAborted(this.currentSignal())

      const tool = toolsByName.get(toolCall.toolName)
      if (!tool) {
        this.transcriptLog.completeToolCall(
          toolCall.toolUseId,
          [{ type: 'text', text: `Tool not found: ${toolCall.toolName}` }],
          true,
        )
        await this.commitTranscript()
        continue
      }

      const input = parseToolInput(toolCall.input)
      const result = await this.invokeToolAsResult(tool, toolCall.toolUseId, input)
      this.transcriptLog.completeToolCall(
        toolCall.toolUseId,
        result.output,
        result.isError ?? false,
        result.metadata ?? result.continuation ?? null,
      )
      await this.runtime.lifecycle?.({
        type: 'after_tool_call',
        agentSessionId: this.agentSessionId,
        state: this.agentState,
        transcript: this.transcriptLog,
        toolCallId: toolCall.toolUseId,
        toolName: toolCall.toolName,
        result,
      })
      await this.commitTranscript()
      stopAfterToolResult ||= result.stopAfterToolResult === true
    }

    return { executed: true, stopAfterToolResult }
  }

  private async invokeTool(
    tool: AgentTool<State>,
    toolCallId: string,
    input: unknown,
  ): Promise<AgentToolInvokeResult> {
    const signal = this.currentSignal()
    return abortable(
      Promise.resolve(
        tool.invoke(
          {
            agentSessionId: this.agentSessionId,
            state: this.agentState,
            cwd: this.cwd,
            toolCallId,
            signal,
            emitProgress: (progress) => {
              this.emit({
                type: 'tool_progress',
                toolCallId,
                toolName: tool.name,
                progress,
              })
            },
          },
          input,
        ),
      ),
      signal,
    )
  }

  private async invokeToolAsResult(
    tool: AgentTool<State>,
    toolCallId: string,
    input: unknown,
  ): Promise<AgentToolInvokeResult> {
    try {
      return await this.invokeTool(tool, toolCallId, input)
    } catch (error) {
      if (isAbortError(error)) throw error
      const normalized = asError(error)
      return {
        output: [{ type: 'text', text: `Tool failed: ${normalized.message}` }],
        isError: true,
        metadata: { error: normalized.message },
      }
    }
  }

  private async executeCompaction(): Promise<boolean> {
    if (this.transcriptLog.pendingToolCalls().length > 0) return false

    const window = this.transcriptLog.findCompactionWindow(this.compactionKeepRecentTokens)
    if (window === null || window.cutPoint <= window.startIndex) return false

    let cutPoint = window.cutPoint
    while (cutPoint > window.startIndex) {
      const compactedBlocks = this.transcriptLog.blocks.slice(window.startIndex, cutPoint)
      const compactedTokens = compactedBlocks.reduce((total, block) => {
        return total + new Transcript([block]).estimateContextTokens()
      }, 0)

      try {
        const summary = await this.generateCompactionSummary(compactedBlocks)
        if (!summary) return false

        const boundary = this.transcriptLog.insertCompactionBoundary(cutPoint, this.model, summary, estimateTokens(summary))
        this.transcriptLog.appendCompactionMarker(this.model, boundary.id, compactedTokens)
        await this.commitTranscript()
        return true
      } catch (error) {
        if (!isContextLengthExceeded(error)) throw error
        const nextCutPoint = nextSmallerCompactionCutPoint(window.startIndex, cutPoint)
        if (nextCutPoint === null) throw error
        cutPoint = nextCutPoint
      }
    }

    return false
  }

  private async generateCompactionSummary(blocks: typeof this.transcriptLog.blocks): Promise<string> {
    const compactTranscript = new Transcript(blocks)
    const request: InferenceRequest = {
      sessionId: this.agentSessionId,
      turnId: this.currentTurnId(),
      requestId: this.idFactory(),
      modelId: this.model.model.id,
      systemPrompt:
        'Summarize the previous conversation for continuation. Preserve user intent, decisions, tool results, and unresolved work.',
      cwd: this.cwd,
      items: compactTranscript.collectInferenceItems(),
      tools: [],
      thinking: this.model.thinking,
      serviceTierId: this.model.serviceTierId ?? null,
      cancel: this.currentSignal(),
    }

    let summary = ''
    for await (const event of this.providerEvents(request)) {
      throwIfAborted(request.cancel)
      if (event.type === 'text_delta') summary += event.text
      if (event.type === 'abort') throw new AbortError()
      if (event.type === 'error') throw new ProviderStreamError(event.message, event.code)
    }
    return summary.trim()
  }

  private async *providerEvents(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const iterator = this.provider.run(request)[Symbol.asyncIterator]()
    let completed = false
    try {
      while (true) {
        const next = await readProviderIterator(iterator, request.cancel)
        if (next.done) {
          completed = true
          return
        }
        yield next.value
      }
    } finally {
      if (!completed) void iterator.return?.().catch(noop)
    }
  }

  private buildInferenceRequest(): InferenceRequest {
    const tools = this.currentTools().map(toToolDefinition)
    return {
      sessionId: this.agentSessionId,
      turnId: this.currentTurnId(),
      requestId: this.idFactory(),
      modelId: this.model.model.id,
      systemPrompt: this.runtime.systemPrompt(this.promptContext()),
      cwd: this.cwd,
      items: this.transcriptLog.collectInferenceItems(),
      tools,
      thinking: this.model.thinking,
      serviceTierId: this.model.serviceTierId ?? null,
      cancel: this.currentSignal(),
    }
  }

  private async applyProviderEvent(event: ProviderEvent): Promise<void> {
    const block = this.transcriptLog.applyProviderEvent(this.model, event)
    if (block) await this.commitTranscript()
  }

  private promptContext(): {
    agentSessionId: string
    state: State
    cwd: string
    transcript: Transcript
  } {
    return {
      agentSessionId: this.agentSessionId,
      state: this.agentState,
      cwd: this.cwd,
      transcript: this.transcriptLog,
    }
  }

  private currentTools(): AgentTool<State>[] {
    return this.runtime.tools({ agentSessionId: this.agentSessionId, state: this.agentState, cwd: this.cwd })
  }

  private isUsageNearLimit(usage: TokenUsage): boolean {
    const contextWindow = this.model.model.contextWindow
    if (contextWindow <= 0) return false
    const usedTokens = usage.inputTokens + usage.outputTokens + usage.cacheReadTokens + usage.cacheWriteTokens
    return usedTokens >= Math.floor(contextWindow * this.compactionThresholdRatio)
  }

  private currentSignal(): AbortSignal {
    if (!this.currentAbortController) throw new Error('AgentSession: no active abort controller')
    return this.currentAbortController.signal
  }

  private currentTurnId(): string {
    if (!this.activeTurnId) throw new Error('AgentSession: no active turn id')
    return this.activeTurnId
  }

  private async recordAbort(): Promise<void> {
    if (this.abortRecorded) return
    this.abortRecorded = true
    for (const toolCall of this.transcriptLog.pendingToolCalls()) {
      this.transcriptLog.completeToolCall(
        toolCall.toolUseId,
        [{ type: 'text', text: `Tool call aborted: ${toolCall.toolName}` }],
        true,
      )
    }
    this.transcriptLog.pushAbort(this.model)
    await this.commitTranscript()
  }

  private removeQueuedMessage(id: string): void {
    const index = this.queued.findIndex((message) => message.id === id)
    if (index === -1) return
    this.queued.splice(index, 1)
    this.emitQueue()
  }

  private setPhase(phase: SessionPhase): void {
    if (this.currentPhase === phase) return
    this.currentPhase = phase
    this.emit({ type: 'phase_changed', phase })
  }

  private emitQueue(): void {
    this.emit({ type: 'queue_changed', queue: this.queuedMessages() })
  }

  private async commitTranscript(): Promise<void> {
    const transcript = this.transcriptLog.snapshot()
    this.emit({ type: 'transcript_changed', transcript })
    await this.store?.saveSnapshot({
      transcript: structuredClone(transcript),
      state: structuredClone(this.agentState),
      phase: this.currentPhase,
      queue: structuredClone(this.queuedMessages()),
      cwd: this.cwd,
      model: structuredClone(this.model),
      harnessName: this.runtime.harnessName,
    })
  }

  private emit(event: SessionEvent): void {
    for (const listener of this.listeners) listener(event)
  }

  private resolveIdleWaiters(): void {
    const waiters = this.idleResolvers
    this.idleResolvers = []
    for (const resolve of waiters) resolve()
  }
}

function toToolDefinition(tool: AgentTool<unknown>): ToolDefinition {
  return {
    name: tool.name,
    description: tool.description,
    inputSchema: tool.inputSchema,
  }
}

function findLastUserTurnIndex(blocks: Array<{ type: string }>): number | null {
  for (let i = blocks.length - 1; i >= 0; i -= 1) {
    if (blocks[i].type === 'user') return i
  }
  return null
}

function parseToolInput(input: string): unknown {
  try {
    return JSON.parse(input)
  } catch {
    return input
  }
}

function textContentSummary(content: UserContentBlock[]): string {
  const text = content
    .map((block) => (block.type === 'text' ? block.text : `[${block.type}]`))
    .join('\n')
    .trim()
  return text.length > 120 ? `${text.slice(0, 117)}...` : text
}

function estimateTokens(text: string): number {
  return Math.ceil(text.length / 4)
}

function nextSmallerCompactionCutPoint(startIndex: number, cutPoint: number): number | null {
  const compactedBlockCount = cutPoint - startIndex
  if (compactedBlockCount <= 1) return null
  return startIndex + Math.max(1, Math.floor(compactedBlockCount / 2))
}

function defaultIdFactory(): string {
  return globalThis.crypto.randomUUID()
}

function noop(): void {}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error))
}

class ProviderStreamError extends Error {
  readonly code: string | null

  constructor(message: string, code: string | null) {
    super(message)
    this.name = 'ProviderStreamError'
    this.code = code
  }
}

function isContextLengthExceeded(error: unknown): boolean {
  return error instanceof ProviderStreamError && error.code === 'context_length_exceeded'
}

async function readProviderIterator(
  iterator: AsyncIterator<ProviderEvent>,
  signal: AbortSignal,
): Promise<IteratorResult<ProviderEvent>> {
  try {
    return await abortable(iterator.next(), signal)
  } catch (error) {
    if (isAbortError(error)) throw error
    const normalized = asError(error)
    return {
      done: false,
      value: {
        type: 'error',
        message: normalized.message,
        code: providerErrorCode(error),
      },
    }
  }
}

function providerErrorCode(error: unknown): string | null {
  if (error !== null && typeof error === 'object' && 'code' in error) {
    const code = (error as { code?: unknown }).code
    if (typeof code === 'string') return code
  }
  return null
}

class AbortError extends Error {
  constructor() {
    super('AgentSession aborted')
    this.name = 'AbortError'
  }
}

function isAbortError(error: unknown): boolean {
  return error instanceof AbortError || (error instanceof Error && error.name === 'AbortError')
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw new AbortError()
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  if (signal.aborted) return Promise.reject(new AbortError())
  return new Promise((resolve, reject) => {
    let settled = false
    const onAbort = (): void => {
      if (settled) return
      settled = true
      signal.removeEventListener('abort', onAbort)
      reject(new AbortError())
    }
    signal.addEventListener('abort', onAbort, { once: true })
    promise.then(
      (value) => {
        if (settled) return
        settled = true
        signal.removeEventListener('abort', onAbort)
        resolve(value)
      },
      (error) => {
        if (settled) return
        settled = true
        signal.removeEventListener('abort', onAbort)
        reject(error)
      },
    )
  })
}
