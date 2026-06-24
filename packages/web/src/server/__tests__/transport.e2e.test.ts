import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { agentSocketUrl, connectAgentClient } from '@demi/web-ui/transport/agent-socket'
import { connectControlClient } from '@demi/web-ui/transport/control-client'
import { parseServerOptions } from '../server-options'
import { startWebServer } from '../serve'
import { createStubProvider } from '../stub-provider'

test('web transport round-trips open/send/stream over websocket', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-transport-'))
  const handle = startWebServer([createStubProvider()], parseServerOptions(['--port', '0', '--cwd', cwd]))

  try {
    const control = await connectControlClient(`ws://localhost:${handle.port}/control`)

    const providers = await control.listProviders()
    expect(providers.map((provider) => provider.id)).toContain('claude-code')

    const models = await control.listModels({ providerId: 'claude-code' })
    expect(models[0]?.id).toBe('stub-model')

    const providerSelection = await control.prepareSession({ providerId: 'claude-code', modelId: 'stub-model' })
    expect(providerSelection).not.toHaveProperty('config')
    expect(providerSelection.model.model.id).toBe('stub-model')

    const client = await connectAgentClient(agentSocketUrl(`http://localhost:${handle.port}`, cwd))
    const seenText: string[] = []
    client.subscribe((event) => {
      if (event.type !== 'transcript_snapshot' && event.type !== 'transcript_patch') return
      for (const block of event.blocks) {
        if (block.type === 'text') seenText.push(block.text)
      }
    })

    await client.open(providerSelection, cwd)
    await client.send([{ type: 'text', text: 'hi' }])

    expect(seenText.some((text) => text.includes('Hello from the stub provider.'))).toBe(true)

    await client.close()
  } finally {
    await handle.stop()
  }
})

test('web backend rejects ordinary HTTP so UI must come from Vite dev server', async () => {
  const cwd = await mkdtemp(join(tmpdir(), 'demi-web-backend-only-'))
  const handle = startWebServer([createStubProvider()], parseServerOptions(['--port', '0', '--cwd', cwd]))

  try {
    const response = await fetch(`${handle.url}/`)

    expect(response.status).toBe(404)
    expect(await response.text()).toContain('Open the Vite dev server')
  } finally {
    await handle.stop()
  }
})
