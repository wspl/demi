import type { AgentDefinition } from '@demi/base-agent'
import { ProviderRegistry, StubProvider, events } from '@demi/provider'
import { RpcHost } from '../index'
import { createStdioHostTransport } from '../stdio-transport'

const providerRegistry = new ProviderRegistry()
providerRegistry.register({
  type: 'child-stub',
  displayName: 'Child Stub',
  createProvider: (config: unknown) => {
    const text = (config as { text: string }).text
    return new StubProvider([[events.text(text), events.response()]])
  },
})

new RpcHost({
  transport: createStdioHostTransport(process.stdin, process.stdout),
  providerRegistry,
  definitions: { test: createDefinition() },
})

function createDefinition(): AgentDefinition<Record<string, never>> {
  return {
    name: 'test',
    initialState: () => ({}),
    systemPrompt: () => 'system',
    tools: () => [],
  }
}
