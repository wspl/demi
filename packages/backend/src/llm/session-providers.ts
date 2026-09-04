import type { ProviderResolver } from '@demicodes/agent'
import { defineProvider, providerRuntime, type AgentProvider, type InferenceRequest, type Provider, type ProviderRun, type ProviderSelection } from '@demicodes/provider'
import type { Host } from '@demicodes/shell'
import type { InstanceMode } from '../auth/identity'
import type { ControlService } from '../storage/control'
import type { ProviderRateLimiter } from '../usage/rate-limit'
import { providerOwner } from '../vault/scope'
import { type ProviderAssembly, type SessionProviderContext, usageAppender } from './assembly'
import { meterRuntime, type MeterOptions } from './metering'

interface SessionProvidersOptions {
  assembly: ProviderAssembly
  control: ControlService
  mode: InstanceMode
  hostFor: (conversationId: string) => Promise<Host>
  rateLimiter: ProviderRateLimiter
}

/** Resolves authorization and credentials at every inference boundary while retaining each session's runtime state. */
export function createSessionProviderResolver(options: SessionProvidersOptions): ProviderResolver {
  return async (providerId, { agentSessionId }) => {
    const conversation = await options.control.getConversation(agentSessionId)
    if (!conversation) throw new Error(`no conversation ${agentSessionId} behind this session`)
    const ownerUserId = providerOwner(options.mode, conversation.userId)
    const resolve = async () => {
      const resolved = await options.assembly.providerFor(providerId)
      return resolved?.entry.ownerUserId === ownerUserId ? resolved : null
    }
    const initial = await resolve()
    if (!initial) return null
    const host = () => options.hostFor(agentSessionId)
    const session: SessionProviderContext = {
      spawn: async (params) => {
        const target = await host()
        const spawn = target.process.spawn
        if (!spawn) throw new Error('this provider needs a machine: the conversation runs hostless')
        return spawn.call(target.process, params)
      },
    }
    const meter: MeterOptions = {
      observe: usageAppender(options.control, { userId: conversation.userId, conversationId: agentSessionId, providerId }),
      beforeRequest: () => options.rateLimiter.take(conversation.userId),
    }
    return defineProvider({
      ...initial.provider,
      createRuntime: (selection) => SessionProviderRuntime.create({ assembly: options.assembly, resolve, host, session, meter }, selection),
    })
  }
}

interface RuntimeOptions {
  assembly: ProviderAssembly
  resolve: () => ReturnType<ProviderAssembly['providerFor']>
  host: () => Promise<Host>
  session: SessionProviderContext
  meter: MeterOptions
}

interface CurrentRuntime {
  base: Provider
  host?: Host
  runtime: AgentProvider
}

class SessionProviderRuntime implements AgentProvider {
  private disposed = false

  constructor(
    private readonly options: RuntimeOptions,
    private selection: ProviderSelection,
    private current?: CurrentRuntime,
  ) {}

  static async create(options: RuntimeOptions, selection: ProviderSelection): Promise<SessionProviderRuntime> {
    const runtime = new SessionProviderRuntime(options, selection)
    await runtime.runtimeForRequest()
    return runtime
  }

  run(request: InferenceRequest): ProviderRun {
    let active: ProviderRun | undefined
    const start = () => this.runtimeForRequest(request)
    return {
      async *[Symbol.asyncIterator]() {
        active = (await start()).run(request)
        yield* active
      },
      get steer() {
        return active?.steer?.bind(active)
      },
    }
  }

  clone(): AgentProvider {
    return new SessionProviderRuntime(this.options, this.selection, this.current && {
      base: this.current.base,
      host: this.current.host,
      runtime: this.current.runtime.clone(),
    })
  }

  async dispose(): Promise<void> {
    this.disposed = true
    await this.current?.runtime.dispose?.()
    this.current = undefined
  }

  private async runtimeForRequest(request?: InferenceRequest): Promise<AgentProvider> {
    const selection: ProviderSelection = request ? {
      ...this.selection,
      model: {
        ...this.selection.model,
        model: { ...this.selection.model.model, id: request.modelId },
        thinking: request.thinking,
        serviceTierId: request.serviceTierId ?? null,
      },
    } : this.selection
    const resolved = await this.options.resolve()
    if (!resolved) {
      await this.current?.runtime.dispose?.()
      this.current = undefined
      throw new Error(`Provider "${this.selection.providerId}" is no longer available to this conversation`)
    }
    const host = resolved.provider.requiresProcessCapableHost ? await this.options.host() : undefined
    if (this.disposed) throw new Error('Provider runtime is disposed')
    if (this.current?.base === resolved.provider && this.current.host === host && this.selection.model.model.id === selection.model.model.id) {
      this.selection = selection
      return this.current.runtime
    }
    const provider = resolved.provider.requiresProcessCapableHost
      ? this.options.assembly.forSession(resolved.entry, this.options.session)
      : resolved.provider
    const runtime = meterRuntime(await providerRuntime(provider, selection), this.options.meter)
    if (this.disposed) {
      await runtime.dispose?.()
      throw new Error('Provider runtime is disposed')
    }
    await this.current?.runtime.dispose?.()
    this.selection = selection
    this.current = { base: resolved.provider, host, runtime }
    return runtime
  }
}
