import { ProviderRegistry, StubProvider, events } from '@demi/provider'
import type { AgentHarness } from '@demi/shell'
import { LocalHost } from '@demi/shell/local-host'
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
  harnesses: { test: createHarness() },
})

function createHarness(): AgentHarness<Record<string, never>> {
  return {
    name: 'test',
    initialState: () => ({}),
    host: (ctx) => new LocalHost(ctx.cwd),
    systemPrompt: () => 'system',
  }
}
