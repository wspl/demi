import { expect, test } from 'bun:test'
import { World } from './world'

test('backend shutdown closes its server even when a resource owner reports failure', async () => {
  const world = await World.create()
  const managed = world.backend.managedHosts
  if (!managed) throw new Error('The scenario requires managed hosts')
  const close = managed.close.bind(managed)
  managed.close = async () => {
    await close()
    throw new Error('injected resource cleanup failure')
  }
  await expect(world.close()).rejects.toThrow('injected resource cleanup failure')
  await expect(fetch(`${world.url}/api/setup`)).rejects.toThrow()
})
