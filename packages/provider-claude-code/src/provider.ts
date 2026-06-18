import type { ToolResultContentBlock } from '@demi/core'
import type { AgentProvider, InferenceItem, InferenceRequest, ProviderDefinition, ProviderEvent } from '@demi/provider'
import { claudeAuthState, claudeRuntimeState } from './cli'
import { controlResponse, inferenceItemToClaudeMessage, requestToInputMessages, toolsListResponse } from './jsonl'
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
    const active = await this.ensureActive(request)
    let keepActiveForContinuation = false
    try {
      const abort = abortPromise(request.cancel, async () => {
        await active.transport.kill()
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
        const next = await Promise.race([active.iterator.next(), abort])
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

        const mapped = mapClaudeStdoutMessage(next.value)
        if (mapped.controlRequest) {
          const handled = await this.handleControlRequest(active, mapped.controlRequest, request)
          if (handled === 'tool-call') {
            const event = controlRequestToToolCall(mapped.controlRequest)
            keepActiveForContinuation = true
            if (event) yield event
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
          await active.transport.wait()
          this.active = null
          return
        }
      }
    } catch (error) {
      if (this.active === active) this.active = null
      await active.transport.kill()
      await active.transport.wait()
      throw error
    } finally {
      if (!keepActiveForContinuation && this.active === active) {
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
    const active: ActiveClaudeRun = { transport, iterator, pendingControlRequest: null, pendingToolUseIds: [] }
    this.active = active

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
      await active.transport.writeJson(
        controlResponse(request.id, {
          protocolVersion: '2024-11-05',
          capabilities: { tools: {} },
          serverInfo: { name: 'demi', version: '0.0.0' },
        }),
      )
      return 'handled'
    }

    if (request.method === 'ping') {
      await active.transport.writeJson(controlResponse(request.id, {}))
      return 'handled'
    }

    if (request.method === 'tools/list') {
      await active.transport.writeJson(controlResponse(request.id, toolsListResponse(inference.tools)))
      return 'handled'
    }

    if (request.method === 'tools/call') {
      if (!controlRequestToToolCall(request)) {
        await active.transport.writeJson(controlResponse(request.id, { error: { message: 'Invalid tools/call request' } }))
        return 'handled'
      }
      active.pendingControlRequest = request
      return 'tool-call'
    }

    await active.transport.writeJson(controlResponse(request.id, { error: { message: `Unsupported method: ${request.method}` } }))
    return 'handled'
  }

  private async writeToolResults(request: InferenceRequest, controlRequest: ClaudeControlRequest): Promise<void> {
    if (!this.active) throw new Error('No active Claude transport')
    const results = request.items.filter((item): item is Extract<InferenceItem, { type: 'tool_result' }> => {
      return item.type === 'tool_result' && item.toolUseId === String(controlRequest.id)
    })
    if (results.length === 0) {
      throw new Error(`Claude Code provider missing tool_result for control_request ${String(controlRequest.id)}`)
    }

    const latest = results[results.length - 1]
    await this.active.transport.writeJson(
      controlResponse(controlRequest.id, {
        content: toolResultContentToText(latest.output),
        isError: latest.isError,
      }),
    )
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

    for (const result of results) {
      const message = inferenceItemToClaudeMessage(result)
      if (message) await this.active.transport.writeJson(message)
    }
  }
}

export function createClaudeCodeProviderDefinition(): ProviderDefinition<unknown> {
  return {
    type: 'claude-code',
    displayName: 'Claude Code',
    auth: { status: claudeAuthState },
    state: claudeRuntimeState,
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

function toolResultContentToText(output: ToolResultContentBlock[]): string {
  return output.map((block) => (block.type === 'text' ? block.text : `[image:${block.source.mediaType}]`)).join('\n')
}

function isToolCallRequested(event: ProviderEvent): event is Extract<ProviderEvent, { type: 'tool_call_requested' }> {
  return event.type === 'tool_call_requested'
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
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
