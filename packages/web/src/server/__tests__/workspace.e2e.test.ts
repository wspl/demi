import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ProviderRegistry } from '@demi/provider'
import { AgentWorkspace } from '@demi/web-ui/agent/workspace'
import { connectControlClient } from '@demi/web-ui/transport/control-client'
import { parseServerOptions } from '../server-options'
import { startWebServer } from '../serve'
import { createStubProviderDefinition } from '../stub-provider'

test('AgentWorkspace drives a conversation through the websocket stack', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-workspace-'))
  const registry = new ProviderRegistry()
  registry.register(createStubProviderDefinition())
  const handle = startWebServer(registry, parseServerOptions(['--port', '0', '--cwd', cwd]))

  try {
    const control = await connectControlClient(`ws://localhost:${handle.port}/control`)
    const workspace = new AgentWorkspace({ baseUrl: `http://localhost:${handle.port}`, control, cwd })

    await workspace.init()

    expect(workspace.order.value.length).toBe(1)
    const id = workspace.activeId.value
    expect(id).not.toBeNull()
    expect(workspace.sessions[id!]?.model.modelId).toBe('stub-model')

    await workspace.send(id!, [{ type: 'text', text: 'hi' }])

    const state = workspace.sessions[id!]!
    expect(state.phase).toBe('idle')
    expect(state.hasContent).toBe(true)
    expect(state.blocks.some((block) => block.type === 'text' && block.text.includes('Hello from the stub provider.'))).toBe(true)

    await workspace.dispose()
  } finally {
    await handle.stop()
  }
})
