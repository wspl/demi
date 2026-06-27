import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { AgentHarness } from '@demicodes/agent'
import { LocalHost } from '@demicodes/host-local'
import { AgentServer } from '../index'
import { createStdioServerTransport } from '../stdio-transport'

const childProvider = defineProvider({
  id: 'child-stub',
  displayName: 'Child Stub',
  createRuntime: () => new StubProvider([[events.text('from child'), events.response()]]),
})

const server = new AgentServer({
  agent: createHarness(),
  providers: [childProvider],
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
