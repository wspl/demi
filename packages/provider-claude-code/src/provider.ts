import { randomUUID } from 'node:crypto'
import type { ToolResultContentBlock } from '@demi/core'
import type { AgentProvider, InferenceItem, InferenceRequest, ProviderDefinition, ProviderEvent } from '@demi/provider'
import { claudeAuthState, claudeRuntimeState } from './cli'
import { controlResponse, inferenceItemToClaudeMessage, requestToInputMessages, toolResultsToClaudeMessage } from './jsonl'
import { listClaudeCodeModels } from './models'
import { controlRequestToToolCall, mapClaudeStdoutMessage, type ClaudeControlRequest } from './output'
import { ClaudeCliTransportFactory, type ClaudeTransport, type ClaudeTransportFactory } from './transport'

export interface ClaudeCodeProviderOptions {
  transportFactory?: ClaudeTransportFactory
  claudePath?: string
  maxBudgetUsd?: number | string | null
}

export interface ClaudeCodeProviderConfig {
  claudePath?: string
  maxBudgetUsd?: number | string | null
}

interface ActiveClaudeRun {
  transport: ClaudeTransport
  iterator: AsyncIterator<unknown>
  pendingControlRequest: ClaudeControlRequest | null
  pendingToolUseIds: string[]
  bufferedMessages: unknown[]
  sdkMcpEnabled: boolean
  hasStreamed: boolean
}

export class ClaudeCodeProvider implements AgentProvider {
  private readonly transportFactory: ClaudeTransportFactory
  private active: ActiveClaudeRun | null = null

  constructor(options: ClaudeCodeProviderOptions = {}) {
    this.transportFactory =
      options.transportFactory ??
      new ClaudeCliTransportFactory({ claudePath: options.claudePath, maxBudgetUsd: options.maxBudgetUsd })
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    let active: ActiveClaudeRun | null = null
    let keepActiveForContinuation = false
    try {
      active = await this.ensureActive(request)
      const run = active
      const abort = abortPromise(request.cancel, async () => {
        await run.transport.kill()
        this.active = null
      })

      if (active.pendingControlRequest) {
        await this.writeToolResults(request, active.pendingControlRequest)
        active.pendingControlRequest = null
      }
      if (active.pendingToolUseIds.length > 0) {
        await this.writeToolResultMessages(request, active.pendingToolUseIds)
        active.pendingToolUseIds = []
      }

      while (true) {
        const next = await Promise.race([this.nextMessage(run), abort])
        if (next.done) {
          const wasAborted = request.cancel.aborted
          const exit = await active.transport.wait()
          this.active = null
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
          this.active = null
          await active.transport.kill()
          await active.transport.wait()
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
      if (!keepActiveForContinuation && active && this.active === active) {
        this.active = null
        await active.transport.kill()
        await active.transport.wait()
      }
    }
  }

  private async ensureActive(request: InferenceRequest): Promise<ActiveClaudeRun> {
    if (this.active) return this.active

    const transport = await this.transportFactory.start(request)
    const iterable = transport.messages()
    const iterator = iterable[Symbol.asyncIterator]()
    const active: ActiveClaudeRun = {
      transport,
      iterator,
      pendingControlRequest: null,
      pendingToolUseIds: [],
      bufferedMessages: [],
      sdkMcpEnabled: request.tools.length > 0,
      hasStreamed: false,
    }
    this.active = active

    if (request.tools.length > 0) {
      await this.initializeSdkMcp(active, request.systemPrompt)
    }

    for (const message of requestToInputMessages(request)) {
      await transport.writeJson(message)
    }

    return active
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

export function createClaudeCodeProviderDefinition(): ProviderDefinition<unknown> {
  return {
    type: 'claude-code',
    displayName: 'Claude Code',
    auth: { status: claudeAuthState },
    state: claudeRuntimeState,
    listModels: (config) => {
      parseClaudeCodeProviderConfig(config)
      return listClaudeCodeModels()
    },
    createProvider: (config) => new ClaudeCodeProvider(parseClaudeCodeProviderConfig(config)),
  }
}

export function parseClaudeCodeProviderConfig(config: unknown): ClaudeCodeProviderConfig {
  if (config === undefined || config === null) return {}
  if (!isRecord(config)) throw new Error('Claude Code provider config must be an object')

  const parsed: ClaudeCodeProviderConfig = {}
  if (config.claudePath !== undefined) {
    if (typeof config.claudePath !== 'string') throw new Error('Claude Code provider config field "claudePath" must be a string')
    parsed.claudePath = config.claudePath
  }
  if (config.maxBudgetUsd !== undefined) {
    if (config.maxBudgetUsd !== null && typeof config.maxBudgetUsd !== 'string' && typeof config.maxBudgetUsd !== 'number') {
      throw new Error('Claude Code provider config field "maxBudgetUsd" must be a string, number, or null')
    }
    parsed.maxBudgetUsd = config.maxBudgetUsd
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
    return {
      type: 'image',
      data: Buffer.from(block.source.data).toString('base64'),
      mimeType: block.source.mediaType,
    }
  })
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function isMessageType(value: unknown, type: string): boolean {
  return isRecord(value) && value.type === type
}

function abortPromise(
  signal: AbortSignal,
  onAbort: () => Promise<void>,
): Promise<{ done: true; value: undefined }> {
  if (signal.aborted) {
    return onAbort().then(() => ({ done: true, value: undefined }))
  }
  return new Promise((resolve) => {
    signal.addEventListener(
      'abort',
      () => {
        void onAbort().finally(() => resolve({ done: true, value: undefined }))
      },
      { once: true },
    )
  })
}
