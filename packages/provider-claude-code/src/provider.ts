import { abortable, isRecord } from '@demicodes/utils'
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
  toolResultContentToText,
  type ProviderQuota,
} from '@demicodes/provider'
import { FileClaudeCodeAuthStore } from './auth'
import { createClaudeCodeCredentials, openClaudeCodeCredentialPool, PoolAwareClaudeCodeAuthStore } from './credentials'
import { coldStartInputMessages, controlResponse, inferenceItemToClaudeMessage, toolResultsToClaudeMessage } from './jsonl'
import { listClaudeCodeModels } from './models'
import { injectableCliToken } from './oauth'
import { controlRequestToToolCall, mapClaudeStdoutMessage, type ClaudeControlRequest } from './output'
import { createClaudeCodeQuota } from './quota'
import { ClaudeCliTransportFactory, type ClaudeSpawn, type ClaudeTransport, type ClaudeTransportFactory } from './transport'

export interface ClaudeCodeProviderOptions {
  id?: string
  displayName?: string
  claudePath?: string
  models?: ModelPolicy
  /** Demi state root for credential pool (`$DEMI_HOME` / `~/.demi`). */
  stateDir?: string
  /** When true (default), attach multi-credential pool + global switch. */
  credentials?: boolean
  authStore?: import('./auth').ClaudeCodeAuthStore
  /** `Host.process`-shaped spawn the CLI runs through (remote execution targets). */
  spawn?: ClaudeSpawn
  /** Public CLI env overlay applied last (e.g. `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_OAUTH_TOKEN`). */
  env?: Record<string, string>
}

export interface ClaudeCodeRuntimeOptions {
  transportFactory?: ClaudeTransportFactory
  claudePath?: string
  /** Shared with the public Provider shell so stream messages can update quota. */
  quota?: ProviderQuota
  authStore?: import('./auth').ClaudeCodeAuthStore
  /** Returns active credential id for process reuse checks. */
  getActiveCredentialId?: () => Promise<string | null>
  /** `Host.process`-shaped spawn the CLI runs through (remote execution targets). */
  spawn?: ClaudeSpawn
  /** Public CLI env overlay applied last (e.g. `ANTHROPIC_BASE_URL`, `CLAUDE_CODE_OAUTH_TOKEN`). */
  env?: Record<string, string>
}

export interface ClaudeCodeProviderConfig {
  claudePath?: string
}

interface ActiveClaudeRun {
  transport: ClaudeTransport
  iterator: AsyncIterator<unknown>
  pendingControlRequest: ClaudeControlRequest | null
  pendingSdkControlRequests: Map<string, ClaudeControlRequest>
  pendingSdkToolCalls: ToolCallEvent[]
  collectingSdkToolCalls: Map<string, ToolCallEvent>
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
  /** Active credential id when the process was spawned (null = vendor default). */
  credentialId: string | null
  /** How many `user_message` items have been delivered to this process so far. */
  sentUserMessageCount: number
  /** Signature of the first user message, used to detect a rewritten transcript (compaction). */
  firstUserSig: string | null
}

type ToolCallEvent = Extract<ProviderEvent, { type: 'tool_call_requested' }>

export class ClaudeCodeProvider implements AgentProvider {
  private readonly cloneOptions: ClaudeCodeRuntimeOptions
  private readonly transportFactory: ClaudeTransportFactory
  private readonly quota: ProviderQuota | null
  private readonly getActiveCredentialId: (() => Promise<string | null>) | null
  private active: ActiveClaudeRun | null = null

  constructor(options: ClaudeCodeRuntimeOptions = {}) {
    this.cloneOptions = options
    this.transportFactory =
      options.transportFactory ??
      new ClaudeCliTransportFactory({
        claudePath: options.claudePath,
        spawn: options.spawn,
        env: options.env,
        resolveOAuthAccessToken: options.authStore
          ? async () => {
              try {
                return injectableCliToken(await options.authStore!.resolveAccess())
              } catch {
                return null
              }
            }
          : undefined,
      })
    this.quota = options.quota ?? null
    this.getActiveCredentialId = options.getActiveCredentialId ?? null
  }

  clone(): AgentProvider {
    return new ClaudeCodeProvider({
      ...this.cloneOptions,
      transportFactory: this.transportFactory,
    })
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
          if (handled === 'sdk-tool-call') {
            const pending = active.pendingSdkControlRequests.get(mapped.controlRequest.toolUseId ?? '')
            const event = pending ? controlRequestToToolCall(pending) : null
            if (event?.type === 'tool_call_requested') {
              active.pendingSdkToolCalls = [event]
              keepActiveForContinuation = true
              yield event
            }
            return
          }
          continue
        }

        const toolUseIds = mapped.events.filter(isToolCallRequested).map((event) => event.toolUseId)
        if (active.sdkMcpEnabled) {
          for (const event of mapped.events) {
            if (event.type === 'tool_call_requested') {
              active.collectingSdkToolCalls.set(event.toolUseId, event)
              continue
            }
            yield event
          }
          if (isStreamMessageStop(raw) && active.collectingSdkToolCalls.size > 0) {
            active.pendingSdkToolCalls = [...active.collectingSdkToolCalls.values()]
            active.collectingSdkToolCalls.clear()
            keepActiveForContinuation = true
            for (const event of active.pendingSdkToolCalls) yield event
            return
          }
          if (mapped.terminal) {
            keepActiveForContinuation = true
            return
          }
          continue
        }
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
    const credentialId = this.getActiveCredentialId ? await this.getActiveCredentialId() : null
    const existing = this.active
    if (
      existing &&
      existing.sessionId === request.sessionId &&
      existing.modelId === request.modelId &&
      existing.thinkingSig === thinkingSignature(request) &&
      existing.credentialId === credentialId
    ) {
      const hasPendingToolCall =
        existing.pendingControlRequest !== null ||
        existing.pendingSdkToolCalls.length > 0 ||
        existing.pendingToolUseIds.length > 0
      if (hasPendingToolCall || !itemsDiverged(existing, request.items)) {
        await this.sendContinuation(existing, request)
        return existing
      }
    }
    if (existing) await this.disposeActive(existing)
    return this.coldStart(request, credentialId)
  }

  private async coldStart(request: InferenceRequest, credentialId: string | null): Promise<ActiveClaudeRun> {
    const transport = await this.transportFactory.start(request)
    const iterator = transport.messages()[Symbol.asyncIterator]()
    const active: ActiveClaudeRun = {
      transport,
      iterator,
      pendingControlRequest: null,
      pendingSdkControlRequests: new Map(),
      pendingSdkToolCalls: [],
      collectingSdkToolCalls: new Map(),
      pendingToolUseIds: [],
      bufferedMessages: [],
      sdkMcpEnabled: request.tools.length > 0,
      hasStreamed: false,
      sessionId: request.sessionId,
      modelId: request.modelId,
      thinkingSig: thinkingSignature(request),
      credentialId,
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
    if (active.pendingSdkToolCalls.length > 0) {
      await this.writeSdkMcpToolResults(active, request)
      active.pendingSdkToolCalls = []
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
  ): Promise<'handled' | 'tool-call' | 'sdk-tool-call'> {
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
      if (request.protocol === 'sdk-mcp') {
        request.toolUseId ??= `mcp-control-${randomUUID()}`
        const normalized = { ...request, toolUseId: request.toolUseId }
        active.pendingSdkControlRequests.set(normalized.toolUseId, normalized)
        return active.hasStreamed ? 'handled' : 'sdk-tool-call'
      }
      active.pendingControlRequest = { ...request, toolUseId: `mcp-control-${randomUUID()}` }
      return 'tool-call'
    }

    await this.writeControlError(active, request, `Unsupported method: ${request.method}`)
    return 'handled'
  }

  private async writeSdkMcpToolResults(
    active: ActiveClaudeRun,
    request: InferenceRequest,
  ): Promise<void> {
    const results = new Map(
      request.items
        .filter((item): item is Extract<InferenceItem, { type: 'tool_result' }> => item.type === 'tool_result')
        .map((item) => [item.toolUseId, item]),
    )
    const expected = active.pendingSdkToolCalls.map((event) => event.toolUseId)
    const missing = expected.filter((toolUseId) => !results.has(toolUseId))
    if (missing.length > 0) {
      throw new Error(`Claude Code provider missing tool_result for SDK MCP tool_use ${missing.join(', ')}`)
    }

    const remaining = new Set(expected)
    while (remaining.size > 0) {
      let responded = false
      for (const toolUseId of remaining) {
        const controlRequest = active.pendingSdkControlRequests.get(toolUseId)
        if (!controlRequest) continue
        await this.writeToolResults(request, controlRequest)
        active.pendingSdkControlRequests.delete(toolUseId)
        remaining.delete(toolUseId)
        responded = true
      }
      if (remaining.size === 0) return
      if (responded) continue

      const next = await abortable(active.iterator.next(), request.cancel)
      if (next.done) {
        throw new Error(
          `Claude Code exited before requesting SDK MCP tool result for ${[...remaining].join(', ')}`,
        )
      }
      this.observeQuotaFromMessage(next.value)
      const mapped = mapClaudeStdoutMessage(next.value, {
        ignoreAssistantContent: true,
        ignoreAssistantToolUse: true,
      })
      if (mapped.controlRequest) {
        const handled = await this.handleControlRequest(active, mapped.controlRequest, request)
        if (handled === 'tool-call') {
          throw new Error('Claude Code emitted a legacy tool call while awaiting SDK MCP results')
        }
        continue
      }
      active.bufferedMessages.push(next.value)
    }
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
  const enableCredentials = options.credentials ?? options.authStore === undefined
  const pool = !options.authStore && enableCredentials ? openClaudeCodeCredentialPool({ stateDir: options.stateDir }) : null
  const authStore =
    options.authStore ??
    (pool ? new PoolAwareClaudeCodeAuthStore(pool) : new FileClaudeCodeAuthStore())

  const quota = createClaudeCodeQuota({
    providerId: id,
    resolveAccess: async () => {
      try {
        return await authStore.resolveAccess()
      } catch {
        return null
      }
    },
  })

  const credentialsApi = pool
    ? createClaudeCodeCredentials(pool, authStore, { quota })
    : undefined

  const runtimeOptions: ClaudeCodeRuntimeOptions = {
    claudePath: options.claudePath,
    quota,
    authStore,
    getActiveCredentialId: pool ? () => pool.getActiveId() : undefined,
    spawn: options.spawn,
    env: options.env,
  }

  return defineProvider({
    id,
    displayName,
    // The CLI transport spawns a real process on the session's execution target.
    requiresProcessCapableHost: true,
    auth: { status: () => authStore.status() },
    quota,
    ...(credentialsApi ? { credentials: credentialsApi } : {}),
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


function toolResultContentToMcp(output: ToolResultContentBlock[]): Array<Record<string, unknown>> {
  return output.map((block) => {
    if (block.type === 'text') return { type: 'text', text: block.text }
    // Claude/MCP has no video content type; degrade defensively (catalog marks video unsupported).
    if (block.type === 'video') return { type: 'text', text: `[video:${block.source.mediaType}]` }
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

function isStreamMessageStop(value: unknown): boolean {
  return (
    isRecord(value) &&
    value.type === 'stream_event' &&
    isRecord(value.event) &&
    value.event.type === 'message_stop'
  )
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
