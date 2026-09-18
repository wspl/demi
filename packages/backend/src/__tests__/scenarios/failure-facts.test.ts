import { afterAll, beforeAll, expect, test } from 'bun:test'
import type { Block, ProviderFailureFacts } from '@demicodes/core'
import { waitFor } from '@demicodes/utils'
import { World } from './world'

// Failure facts (backend.md § Failure facts): a failed turn keeps the vendor's
// record as it arrived, and the backend sends what the provider reads out of it
// beside the transcript, never inside the stored block.

let world: World

beforeAll(async () => {
  world = await World.create({})
})

afterAll(async () => {
  await world.close()
})

test('the history route sends what the provider reads from each error block\'s record', async () => {
  const driver = await world.conversation('cloud')
  const upstream = '{"type":"error","error":{"message":"The usage limit has been reached","resets_at":1790062659},"status_code":429}'
  const read: Array<{ upstream: string | undefined; receivedAt: string }> = []
  world.model.readFailure = (diagnostics, receivedAt) => {
    read.push({ upstream: diagnostics.upstream, receivedAt })
    return { retryAt: new Date(1790062659 * 1000).toISOString() }
  }
  const { done } = driver.startTurn({ model: [[{
    type: 'error',
    message: 'The usage limit has been reached',
    code: 'auth_expired',
    diagnostics: { source: 'stream', upstream },
  }]] })
  await done
  await waitFor(() => driver.transcript().at(-1)?.type === 'error', undefined, { timeoutMs: 5_000 })
  const failed = driver.transcript().at(-1)!
  // The record is stored as it came.
  expect(failed).toMatchObject({ type: 'error', diagnostics: { source: 'stream', upstream } })

  const history = await world.api<{
    blocks: Block[]
    failures: Record<string, ProviderFailureFacts>
    subagents: Array<{ failures: Record<string, ProviderFailureFacts> }>
  }>(`/api/conversations/${driver.id}/transcript`)
  expect(history.failures).toEqual({
    [failed.id]: { retryAt: new Date(1790062659 * 1000).toISOString() },
  })
  // The provider read the stored record, counting from when it was recorded.
  expect(read.at(-1)).toEqual({ upstream, receivedAt: failed.createdAt })
  // The stored block carries no facts of its own.
  expect(history.blocks.find((block) => block.id === failed.id)).not.toHaveProperty('failures')
  world.model.readFailure = () => ({ retryAt: null })
})
