import { defineProvider } from '@demicodes/provider'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { AgentHarness } from '@demicodes/agent'
import { hostlessShellFactory } from '@demicodes/host-virtual/testing'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { AgentServer } from '../index'
import { createStdioServerTransport } from '../protocol/stdio-transport'

const childProvider = defineProvider({
  id: 'child-stub',
  displayName: 'Child Stub',
  createRuntime: () => new StubProvider([[events.text('from child'), events.response()]]),
})

const server = new AgentServer({
  shellEnvironment: hostlessShellFactory,
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
