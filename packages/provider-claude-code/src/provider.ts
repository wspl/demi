import { isRecord } from '@demicodes/utils'
import { randomUUID } from 'node:crypto'
import type { ToolResultContentBlock } from '@demicodes/core'
import {
  applyModelPolicy,
  defineProvider,
  type AgentProvider,
  type InferenceItem,
  type InferenceRequest,
  type ModelPolicy,
  type Provider,
  type ProviderEvent,
  type ProviderQuota,
} from '@demicodes/provider'
import { coldStartInputMessages, controlResponse, inferenceItemToClaudeMessage, toolResultsToClaudeMessage } from './jsonl'
import { listClaudeCodeModels } from './models'
import { controlRequestToToolCall, mapClaudeStdoutMessage, type ClaudeControlRequest } from './output'
import { createClaudeCodeQuota } from './quota'
import { ClaudeCliTransportFactory, type ClaudeTransport, type ClaudeTransportFactory } from './transport'

export interface ClaudeCodeProviderOptions {
  id?: string
  displayName?: string
  claudePath?: string
  models?: ModelPolicy
}

export interface ClaudeCodeRuntimeOptions {
  transportFactory?: ClaudeTransportFactory
  claudePath?: string
  /** Shared with the public Provider shell so stream messages can update quota. */
  quota?: ProviderQuota
}

export interface ClaudeCodeProviderConfig {
  claudePath?: string
}

interface ActiveClaudeRun {
  transport: ClaudeTransport
  iterator: AsyncIterator<unknown>
  pendingControlRequest: ClaudeControlRequest | null
  pendingToolUseIds: string[]
  bufferedMessages: unknown[]
  sdkMcpEnabled: boolean
  hasStreamed: boolean
  /** Session this live process belongs to; a different session forces a cold restart. */
  sessionId: string
  /** Model + thinking effort this process was spawned with; switching either forces a cold restart
   *  (both `--model` and `--effort` are fixed per process). */
  modelId: string
  thinkingSig: string
  /** How many `user_message` items have been delivered to this process so far. */
  sentUserMessageCount: number
  /** Signature of the first user message, used to detect a rewritten transcript (compaction). */
  firstUserSig: string | null
}

export class ClaudeCodeProvider implements AgentProvider {
  private readonly transportFactory: ClaudeTransportFactory
  private readonly quota: ProviderQuota | null
  private active: ActiveClaudeRun | null = null

  constructor(options: ClaudeCodeRuntimeOptions = {}) {
    this.transportFactory =
      options.transportFactory ?? new ClaudeCliTransportFactory({ claudePath: options.claudePath })
    this.quota = options.quota ?? null
  }

  private observeQuotaFromMessage(message: unknown): void {
    try {
      this.quota?.observeResponse?.({ body: message })
    } catch {
      // Quota observation must never break inference.
    }
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    let active: ActiveClaudeRun | null = null
    let keepActiveForContinuation = false
    const signal = request.cancel
    let abortListener: (() => void) | null = null
    try {
      active = await this.ensureActiveForRequest(request)
      const run = active
      const onAbort = async (): Promise<void> => {
        await run.transport.kill()
        if (this.active === run) this.active = null
      }
      const abort = new Promise<{ done: true; value: undefined }>((resolve) => {
        if (signal.aborted) {
          void onAbort().then(() => resolve({ done: true, value: undefined }))
          return
        }
        abortListener = () => {
          void onAbort().finally(() => resolve({ done: true, value: undefined }))
        }
        signal.addEventListener('abort', abortListener, { once: true })
      })

      while (true) {
        const next = await Promise.race([this.nextMessage(run), abort])
        if (next.done) {
          const wasAborted = signal.aborted
          const exit = await active.transport.wait()
          if (this.active === active) this.active = null
          if (!wasAborted && exit.exitCode !== 0) {
            yield {
              type: 'error',
              message: active.transport.stderrText() || `Claude Code exited with code ${exit.exitCode}`,
              code: exit.signal ?? null,
            }
          }
          return
        }

        const raw = next.value
        this.observeQuotaFromMessage(raw)
        const ignoreAssistantContent = active.hasStreamed && isMessageType(raw, 'assistant')
        const mapped = mapClaudeStdoutMessage(raw, {
          ignoreAssistantContent,
          ignoreAssistantToolUse: active.sdkMcpEnabled,
        })
        if (isMessageType(raw, 'stream_event')) active.hasStreamed = true
        if (mapped.controlRequest) {
          const handled = await this.handleControlRequest(active, mapped.controlRequest, request)
          if (handled === 'tool-call') {
            const event = active.pendingControlRequest ? controlRequestToToolCall(active.pendingControlRequest) : null
            keepActiveForContinuation = true
            if (event) {
              yield event
            }
            return
          }
          continue
        }

        const toolUseIds = mapped.events.filter(isToolCallRequested).map((event) => event.toolUseId)
        if (toolUseIds.length > 0) active.pendingToolUseIds = toolUseIds
        for (const event of mapped.events) {
          if (event.type === 'tool_call_requested') keepActiveForContinuation = true
          yield event
        }
        if (toolUseIds.length > 0) return

        if (mapped.terminal) {
          // End of a model turn. The CLI process stays alive in streaming-input mode, keeping
          // the full conversation (and the live SDK-MCP session) in its own native context, so
          // the next turn only needs to send the new user message — no restart, no replay.
          keepActiveForContinuation = true
          return
        }
      }
    } catch (error) {
      const cleanup = active ?? this.active
      if (cleanup) {
        if (this.active === cleanup) this.active = null
        await cleanup.transport.kill()
        await cleanup.transport.wait()
      }
      throw error
    } finally {
      if (abortListener) signal.removeEventListener('abort', abortListener)
      if (!keepActiveForContinuation && active && this.active === active) {
        this.active = null
        await active.transport.kill()
        await active.transport.wait()
      }
    }
  }

  async dispose(): Promise<void> {
    const active = this.active
    if (!active) return
    this.active = null
    await active.transport.kill()
    await active.transport.wait()
  }

  /**
   * Returns the live process for this request, reusing the one kept alive from the previous
   * turn whenever possible. Reuse delivers only the *new* input (a pending tool result and/or
   * a freshly appended user message); a cold start is taken only when there is no live process,
   * the session changed, or the transcript was rewritten underneath us (compaction).
   */
  private async ensureActiveForRequest(request: InferenceRequest): Promise<ActiveClaudeRun> {
    const existing = this.active
    if (
      existing &&
      existing.sessionId === request.sessionId &&
      existing.modelId === request.modelId &&
      existing.thinkingSig === thinkingSignature(request)
    ) {
      const hasPendingToolCall =
        existing.pendingControlRequest !== null || existing.pendingToolUseIds.length > 0
      if (hasPendingToolCall || !itemsDiverged(existing, request.items)) {
        await this.sendContinuation(existing, request)
        return existing
      }
    }
    if (existing) await this.disposeActive(existing)
    return this.coldStart(request)
  }

  private async coldStart(request: InferenceRequest): Promise<ActiveClaudeRun> {
    const transport = await this.transportFactory.start(request)
    const iterator = transport.messages()[Symbol.asyncIterator]()
    const active: ActiveClaudeRun = {
      transport,
      iterator,
      pendingControlRequest: null,
      pendingToolUseIds: [],
      bufferedMessages: [],
      sdkMcpEnabled: request.tools.length > 0,
      hasStreamed: false,
      sessionId: request.sessionId,
      modelId: request.modelId,
      thinkingSig: thinkingSignature(request),
      sentUserMessageCount: 0,
      firstUserSig: null,
    }
    this.active = active

    if (request.tools.length > 0) {
      await this.initializeSdkMcp(active, request.systemPrompt)
    }

    for (const message of coldStartInputMessages(request.items)) {
      await transport.writeJson(message)
    }
    active.sentUserMessageCount = countUserMessages(request.items)
    active.firstUserSig = firstUserSignature(request.items)

    return active
  }

  /** Feeds a reused process only the input it has not seen: pending tool results, then any new user turns. */
  private async sendContinuation(active: ActiveClaudeRun, request: InferenceRequest): Promise<void> {
    if (active.pendingControlRequest) {
      await this.writeToolResults(request, active.pendingControlRequest)
      active.pendingControlRequest = null
    }
    if (active.pendingToolUseIds.length > 0) {
      await this.writeToolResultMessages(request, active.pendingToolUseIds)
      active.pendingToolUseIds = []
    }

    const userCount = countUserMessages(request.items)
    if (userCount > active.sentUserMessageCount) {
      const userItems = request.items.filter(
        (item): item is Extract<InferenceItem, { type: 'user_message' | 'user_steer' }> =>
          item.type === 'user_message' || item.type === 'user_steer',
      )
      for (const item of userItems.slice(active.sentUserMessageCount)) {
        const message = inferenceItemToClaudeMessage(item)
        if (message) await active.transport.writeJson(message)
      }
      active.sentUserMessageCount = userCount
      if (active.firstUserSig === null) active.firstUserSig = firstUserSignature(request.items)
    }
  }

  private async disposeActive(active: ActiveClaudeRun): Promise<void> {
    if (this.active === active) this.active = null
    await active.transport.kill()
    await active.transport.wait()
  }

  private async handleControlRequest(
    active: ActiveClaudeRun,
    request: ClaudeControlRequest,
    inference: InferenceRequest,
  ): Promise<'handled' | 'tool-call'> {
    if (request.method === 'initialize') {
      await this.writeControlResponse(active, request, {
        protocolVersion: '2024-11-05',
        capabilities: { tools: {} },
        serverInfo: { name: 'demi', version: '0.0.0' },
      })
      return 'handled'
    }

    if (request.method === 'ping') {
      await this.writeControlResponse(active, request, {})
      return 'handled'
    }

    if (request.method === 'notifications/initialized') {
      await this.writeControlResponse(active, request, {})
      return 'handled'
    }

    if (request.method === 'tools/list') {
      await this.writeControlResponse(active, request, {
        tools: inference.tools.map((tool) => ({
          name: tool.name,
          description: tool.description,
          inputSchema: tool.inputSchema,
        })),
      })
      return 'handled'
    }

    if (request.method === 'tools/call') {
      if (!controlRequestToToolCall(request)) {
        await this.writeControlError(active, request, 'Invalid tools/call request')
        return 'handled'
      }
      active.pendingControlRequest = { ...request, toolUseId: `mcp-control-${randomUUID()}` }
      return 'tool-call'
    }

    await this.writeControlError(active, request, `Unsupported method: ${request.method}`)
    return 'handled'
  }

  private async writeToolResults(request: InferenceRequest, controlRequest: ClaudeControlRequest): Promise<void> {
    if (!this.active) throw new Error('No active Claude transport')
    const expectedToolUseId = controlRequest.toolUseId ?? String(controlRequest.id)
    const results = request.items.filter((item): item is Extract<InferenceItem, { type: 'tool_result' }> => {
      return item.type === 'tool_result' && item.toolUseId === expectedToolUseId
    })
    if (results.length === 0) {
      throw new Error(`Claude Code provider missing tool_result for control_request ${String(controlRequest.id)}`)
    }

    const latest = results[results.length - 1]
    if (controlRequest.protocol === 'sdk-mcp') {
      await this.writeControlResponse(this.active, controlRequest, {
        content: toolResultContentToMcp(latest.output),
        isError: latest.isError,
      })
      return
    }

    await this.writeControlResponse(this.active, controlRequest, {
      content: toolResultContentToText(latest.output),
      isError: latest.isError,
    })
  }

  private async writeToolResultMessages(request: InferenceRequest, toolUseIds: string[]): Promise<void> {
    if (!this.active) throw new Error('No active Claude transport')
    const pending = new Set(toolUseIds)
    const results = request.items.filter((item): item is Extract<InferenceItem, { type: 'tool_result' }> => {
      return item.type === 'tool_result' && pending.has(item.toolUseId)
    })
    const found = new Set(results.map((item) => item.toolUseId))
    const missing = toolUseIds.filter((id) => !found.has(id))
    if (missing.length > 0) {
      throw new Error(`Claude Code provider missing tool_result for tool_use ${missing.join(', ')}`)
    }

    await this.active.transport.writeJson(toolResultsToClaudeMessage(results))
  }

  private async initializeSdkMcp(active: ActiveClaudeRun, systemPrompt: string): Promise<void> {
    const requestId = randomUUID()
    await active.transport.writeJson({
      type: 'control_request',
      request_id: requestId,
      request: {
        subtype: 'initialize',
        sdkMcpServers: ['main'],
        systemPrompt,
      },
    })

    while (true) {
      const next = await active.iterator.next()
      if (next.done) throw new Error('Claude Code exited before SDK MCP initialization completed')
      if (isControlResponseFor(next.value, requestId)) return
      active.bufferedMessages.push(next.value)
    }
  }

  private nextMessage(active: ActiveClaudeRun): Promise<IteratorResult<unknown>> {
    const value = active.bufferedMessages.shift()
    if (value !== undefined) return Promise.resolve({ done: false, value })
    return active.iterator.next()
  }

  private async writeControlResponse(active: ActiveClaudeRun, request: ClaudeControlRequest, result: unknown): Promise<void> {
    if (request.protocol === 'sdk-mcp') {
      await active.transport.writeJson({
        type: 'control_response',
        response: {
          subtype: 'success',
          request_id: request.outerRequestId,
          response: {
            mcp_response: {
              jsonrpc: '2.0',
              id: request.id,
              result,
            },
          },
        },
      })
      return
    }

    await active.transport.writeJson(controlResponse(request.id, result))
  }

  private async writeControlError(active: ActiveClaudeRun, request: ClaudeControlRequest, message: string): Promise<void> {
    if (request.protocol === 'sdk-mcp') {
      await active.transport.writeJson({
        type: 'control_response',
        response: {
          subtype: 'success',
          request_id: request.outerRequestId,
          response: {
            mcp_response: {
              jsonrpc: '2.0',
              id: request.id,
              error: { code: -32601, message },
            },
          },
        },
      })
      return
    }

    await active.transport.writeJson(controlResponse(request.id, { error: { message } }))
  }
}

export function createClaudeCodeProvider(options: ClaudeCodeProviderOptions = {}): Provider {
  const id = options.id ?? 'claude-code'
  const displayName = options.displayName ?? 'Claude Code'
  const quota = createClaudeCodeQuota({ providerId: id })
  const runtimeOptions: ClaudeCodeRuntimeOptions = {
    claudePath: options.claudePath,
    quota,
  }

  return defineProvider({
    id,
    displayName,
    auth: { status: () => ({ status: 'unknown', message: 'Auth is checked when a Claude Code request runs' }) },
    quota,
    state: () => ({ status: 'unknown', message: 'Runtime is checked when a Claude Code request runs' }),
    listModels: async () => {
      const catalog = await listClaudeCodeModels()
      return applyModelPolicy(catalog, id, options.models)
    },
    createRuntime: () => new ClaudeCodeProvider(runtimeOptions),
  })
}

export function parseClaudeCodeProviderConfig(config: unknown): ClaudeCodeProviderConfig {
  if (config === undefined || config === null) return {}
  if (!isRecord(config)) throw new Error('Claude Code provider config must be an object')

  const parsed: ClaudeCodeProviderConfig = {}
  if (config.claudePath !== undefined) {
    if (typeof config.claudePath !== 'string') throw new Error('Claude Code provider config field "claudePath" must be a string')
    parsed.claudePath = config.claudePath
  }
  return parsed
}

function isToolCallRequested(event: ProviderEvent): event is Extract<ProviderEvent, { type: 'tool_call_requested' }> {
  return event.type === 'tool_call_requested'
}

function isControlResponseFor(value: unknown, requestId: string): boolean {
  if (!isRecord(value) || value.type !== 'control_response' || !isRecord(value.response)) return false
  return value.response.request_id === requestId && value.response.subtype === 'success'
}

function toolResultContentToText(output: ToolResultContentBlock[]): string {
  return output.map((block) => (block.type === 'text' ? block.text : `[image:${block.source.mediaType}]`)).join('\n')
}

function toolResultContentToMcp(output: ToolResultContentBlock[]): Array<Record<string, unknown>> {
  return output.map((block) => {
    if (block.type === 'text') return { type: 'text', text: block.text }
    // block.source.data is already a base64 string (Base64ImageSource); MCP image
    // content wants base64 in `data`, so pass it through — re-encoding would double it.
    return {
      type: 'image',
      data: block.source.data,
      mimeType: block.source.mediaType,
    }
  })
}

function isMessageType(value: unknown, type: string): boolean {
  return isRecord(value) && value.type === type
}

function thinkingSignature(request: InferenceRequest): string {
  return JSON.stringify(request.thinking ?? null)
}

function countUserMessages(items: InferenceItem[]): number {
  let count = 0
  for (const item of items) if (item.type === 'user_message' || item.type === 'user_steer') count += 1
  return count
}

function firstUserSignature(items: InferenceItem[]): string | null {
  const first = items.find((item) => item.type === 'user_message')
  if (!first || first.type !== 'user_message') return null
  // Cheap content fingerprint that avoids hashing large base64 image/document payloads.
  return first.content.map((block) => (block.type === 'text' ? `t:${block.text}` : block.type)).join('|')
}

/**
 * True when the transcript no longer extends what the live process has already consumed —
 * i.e. user turns were removed or the leading user message changed (compaction rewrote the
 * history). In that case the process must be cold-restarted from the rewritten transcript.
 */
function itemsDiverged(active: ActiveClaudeRun, items: InferenceItem[]): boolean {
  if (countUserMessages(items) < active.sentUserMessageCount) return true
  if (active.firstUserSig !== null && firstUserSignature(items) !== active.firstUserSig) return true
  return false
}
