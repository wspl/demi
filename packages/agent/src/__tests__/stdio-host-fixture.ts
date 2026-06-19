import { ProviderRegistry } from '@demi/provider'
import { StubProvider, events } from '@demi/provider/testing'
import type { AgentHarness } from '@demi/agent'
import { LocalHost } from '@demi/shell/local-host'
import { AgentServer } from '../index'
import { createStdioServerTransport } from '../stdio-transport'

const providerRegistry = new ProviderRegistry()
providerRegistry.register({
  type: 'child-stub',
  displayName: 'Child Stub',
  createProvider: (config: unknown) => {
    const text = (config as { text: string }).text
    return new StubProvider([[events.text(text), events.response()]])
  },
})

const server = new AgentServer({
  agent: createHarness(),
  providerRegistry,
})
server.attachTransport(createStdioServerTransport(process.stdin, process.stdout))

function createHarness(): AgentHarness<Record<string, never>> {
  return {
    name: 'test',
    initialState: () => ({}),
    host: (ctx) => new LocalHost(ctx.cwd),
    systemPrompt: () => 'system',
  }
}
