import { AbortError, abortable, asError, isAbortError, parseJsonOrString, throwIfAborted } from '@demicodes/utils'
import type { ModelSelection, TokenUsage } from '@demicodes/core'
import type { AgentProvider, InferenceRequest, ProviderEvent, ProviderRun, ToolDefinition } from '@demicodes/provider'
import { Transcript } from './transcript'
import { ProviderStreamError } from './provider-stream-error'
import type { ActiveTurnPhase } from './session'
import type { AgentHarnessRuntime, AgentTool, AgentToolInvokeResult, SessionEvent } from './types'

const MAX_AUTO_COMPACTIONS_PER_TURN = 3

/**
 * What ProviderTurnLoop needs from its owning session. This is the session's hot path, so the
 * coupling is wide and intentional: the loop drives the provider stream, applies events to the
 * transcript, executes tools, and triggers steer materialization / auto-recover compaction, all
 * against the session's live turn state — exposed here as an explicit contract.
 */
export interface ProviderTurnLoopHost<State> {
  readonly transcript: Transcript
  readonly model: ModelSelection
  readonly provider: AgentProvider
  readonly runtime: AgentHarnessRuntime<State>
  readonly agentSessionId: string
  readonly cwd: string
  readonly agentState: State
  readonly thresholdRatio: number
  readonly steerContinuationCount: number
  currentSignal(): AbortSignal
  currentTurnId(): string
  nextRequestId(): string
  promptContext(): { agentSessionId: string; state: State; cwd: string; transcript: Transcript }
  getActiveTurnPhase(): ActiveTurnPhase | null
  setActiveTurnPhase(phase: ActiveTurnPhase | null): void
  getActiveProviderRun(): ProviderRun | null
  setActiveProviderRun(run: ProviderRun | null): void
  streamProvider(request: InferenceRequest, run: ProviderRun): AsyncIterable<ProviderEvent>
  runCompaction(): Promise<boolean>
  runWithCompactingPhase<T>(fn: () => Promise<T>): Promise<T>
  commitTranscript(): Promise<void>
  emit(event: SessionEvent): void
  materializeSteersArrivedSince(continuationCount: number): Promise<boolean>
}

/**
 * Runs a provider turn to completion: stream once, materialize steers that arrived, execute any
 * requested tools, and — if usage neared the context limit — auto-compact and resume, looping until
 * the model produces a terminal turn (or a tool asks to stop).
 */
export class ProviderTurnLoop<State> {
  constructor(private readonly host: ProviderTurnLoopHost<State>) {}

  async run(): Promise<void> {
    let autoCompactions = 0
    while (true) {
      throwIfAborted(this.host.currentSignal())
      const steerContinuationBeforeStream = this.host.steerContinuationCount
      const shouldAutoRecover = await this.streamProviderOnce()
      throwIfAborted(this.host.currentSignal())

      if (!shouldAutoRecover) {
        await this.host.materializeSteersArrivedSince(steerContinuationBeforeStream)
      }
      const toolExecution = await this.executePendingTools({ deferSteerMaterialization: shouldAutoRecover })
      if (shouldAutoRecover && autoCompactions < MAX_AUTO_COMPACTIONS_PER_TURN) {
        const tokensBefore = this.host.transcript.estimateContextTokens()
        const compacted = await this.host.runWithCompactingPhase(() => this.host.runCompaction())
        // Only loop if compaction actually shrank the transcript. Otherwise we'd keep compacting
        // our own summaries and pile up resume turns (a storm) until the model rejects the history.
        if (compacted && this.host.transcript.estimateContextTokens() < tokensBefore) {
          autoCompactions += 1
          this.host.transcript.pushResumeTurn(this.host.currentTurnId(), this.host.model)
          await this.host.commitTranscript()
          continue
        }
      }
      if (toolExecution.stopAfterToolResult) return
      if (this.host.steerContinuationCount > steerContinuationBeforeStream) {
        await this.host.materializeSteersArrivedSince(steerContinuationBeforeStream)
        continue
      }
      if (!toolExecution.executed) return
    }
  }

  private async streamProviderOnce(): Promise<boolean> {
    const request = this.buildInferenceRequest()
    const run = this.host.provider.run(request)
    let shouldAutoRecover = false
    this.host.setActiveProviderRun(run)
    this.host.setActiveTurnPhase('provider_streaming')
    try {
      for await (const event of this.host.streamProvider(request, run)) {
        throwIfAborted(request.cancel)
        if (event.type === 'abort') throw new AbortError()
        await this.applyProviderEvent(event)
        if (event.type === 'error') throw new ProviderStreamError(event.message, event.code)
        if (event.type === 'response' && this.isUsageNearLimit(event.usage)) {
          shouldAutoRecover = true
        }
      }
    } finally {
      if (this.host.getActiveProviderRun() === run) this.host.setActiveProviderRun(null)
      if (this.host.getActiveTurnPhase() === 'provider_streaming') this.host.setActiveTurnPhase(null)
    }
    return shouldAutoRecover
  }

  private async executePendingTools(
    options: { deferSteerMaterialization?: boolean } = {},
  ): Promise<{ executed: boolean; stopAfterToolResult: boolean }> {
    const pending = this.host.transcript.pendingToolCalls()
    if (pending.length === 0) return { executed: false, stopAfterToolResult: false }

    const tools = this.currentTools()
    const toolsByName = new Map(tools.map((tool) => [tool.name, tool]))
    let stopAfterToolResult = false
    const previousActivePhase = this.host.getActiveTurnPhase()
    this.host.setActiveTurnPhase('tool_executing')

    try {
      for (const toolCall of pending) {
        throwIfAborted(this.host.currentSignal())
        const steerContinuationBeforeTool = this.host.steerContinuationCount

        const tool = toolsByName.get(toolCall.toolName)
        if (!tool) {
          this.host.transcript.completeToolCall(
            toolCall.toolUseId,
            [{ type: 'text', text: `Tool not found: ${toolCall.toolName}` }],
            true,
          )
          await this.host.commitTranscript()
          if (!options.deferSteerMaterialization) {
            await this.host.materializeSteersArrivedSince(steerContinuationBeforeTool)
          }
          continue
        }

        const input = parseJsonOrString(toolCall.input)
        const result = await this.invokeToolAsResult(tool, toolCall.toolUseId, input)
        this.host.transcript.completeToolCall(
          toolCall.toolUseId,
          result.output,
          result.isError ?? false,
          result.metadata ?? result.continuation ?? null,
        )
        await this.host.runtime.lifecycle?.({
          type: 'after_tool_call',
          agentSessionId: this.host.agentSessionId,
          state: this.host.agentState,
          transcript: this.host.transcript,
          toolCallId: toolCall.toolUseId,
          toolName: toolCall.toolName,
          result,
        })
        await this.host.commitTranscript()
        if (!options.deferSteerMaterialization) {
          await this.host.materializeSteersArrivedSince(steerContinuationBeforeTool)
        }
        stopAfterToolResult ||= result.stopAfterToolResult === true
      }
    } finally {
      this.host.setActiveTurnPhase(previousActivePhase)
    }

    return { executed: true, stopAfterToolResult }
  }

  private async invokeTool(tool: AgentTool<State>, toolCallId: string, input: unknown): Promise<AgentToolInvokeResult> {
    const signal = this.host.currentSignal()
    return abortable(
      Promise.resolve(
        tool.invoke(
          {
            agentSessionId: this.host.agentSessionId,
            state: this.host.agentState,
            cwd: this.host.cwd,
            model: this.host.model,
            toolCallId,
            signal,
            emitProgress: (progress) => {
              this.host.emit({ type: 'tool_progress', toolCallId, toolName: tool.name, progress })
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

  private currentTools(): AgentTool<State>[] {
    return this.host.runtime.tools({
      agentSessionId: this.host.agentSessionId,
      state: this.host.agentState,
      cwd: this.host.cwd,
    })
  }

  private buildInferenceRequest(): InferenceRequest {
    const tools = this.currentTools().map(toToolDefinition)
    return {
      sessionId: this.host.agentSessionId,
      turnId: this.host.currentTurnId(),
      requestId: this.host.nextRequestId(),
      modelId: this.host.model.model.id,
      systemPrompt: this.host.runtime.systemPrompt(this.host.promptContext()),
      cwd: this.host.cwd,
      items: this.host.transcript.collectInferenceItems(),
      tools,
      thinking: this.host.model.thinking,
      serviceTierId: this.host.model.serviceTierId ?? null,
      cancel: this.host.currentSignal(),
    }
  }

  private async applyProviderEvent(event: ProviderEvent): Promise<void> {
    const block = this.host.transcript.applyProviderEvent(this.host.model, event)
    if (block) await this.host.commitTranscript()
  }

  private isUsageNearLimit(usage: TokenUsage): boolean {
    const contextWindow = this.host.model.model.contextWindow
    if (contextWindow <= 0) return false
    const usedTokens = usage.inputTokens + usage.outputTokens + usage.cacheReadTokens + usage.cacheWriteTokens
    return usedTokens >= Math.floor(contextWindow * this.host.thresholdRatio)
  }
}

function toToolDefinition(tool: AgentTool<unknown>): ToolDefinition {
  return {
    name: tool.name,
    description: tool.description,
    inputSchema: tool.inputSchema,
  }
}
