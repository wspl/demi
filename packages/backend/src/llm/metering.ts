import type { TokenUsage } from '@demicodes/core'
import {
  defineProvider,
  providerRuntime,
  type AgentProvider,
  type InferenceRequest,
  type Provider,
  type ProviderFactoryDefinition,
  type ProviderRun,
} from '@demicodes/provider'

/** Called once per provider request with the usage that request reported. */
export type UsageObserver = (usage: TokenUsage, request: { modelId: string }) => void

export interface MeterOptions {
  observe: UsageObserver
  /** Enforcement at the inference entry: throw to refuse the request (e.g. rate limit). */
  beforeRequest?: (request: { modelId: string }) => void
}

/**
 * The metering wrap at the inference entry: usage is observed firsthand from
 * the provider's own `response` events — one call per provider request, which
 * is exactly the ledger granularity — and enforcement runs before a request
 * starts. Everything else (steer, clone, dispose) delegates untouched.
 */
export function meterProvider(inner: Provider, options: MeterOptions): Provider {
  const definition = { ...inner, createRuntime: undefined } as unknown as ProviderFactoryDefinition
  definition.createRuntime = async (selection) => meterRuntime(await providerRuntime(inner, selection), options)
  return defineProvider(definition)
}

function meterRuntime(runtime: AgentProvider, options: MeterOptions): AgentProvider {
  const metered: AgentProvider = {
    run: (request) => {
      options.beforeRequest?.({ modelId: request.modelId })
      return meterRun(runtime.run(request), request, options.observe)
    },
    clone: () => meterRuntime(runtime.clone(), options),
  }
  if (runtime.dispose) metered.dispose = () => runtime.dispose?.()
  return metered
}

function meterRun(run: ProviderRun, request: InferenceRequest, observe: UsageObserver): ProviderRun {
  const metered: ProviderRun = {
    async *[Symbol.asyncIterator]() {
      for await (const event of run) {
        if (event.type === 'response') observe(event.usage, { modelId: request.modelId })
        yield event
      }
    },
  }
  if (run.steer) metered.steer = (input) => run.steer?.(input)
  return metered
}
