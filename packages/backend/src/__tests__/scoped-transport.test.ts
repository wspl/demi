import { afterEach, expect, test } from 'bun:test'
import { createInProcessTransportPair, type BlobStore, type ClientFrame, type ServerFrame } from '@demicodes/agent'
import { waitFor } from '@demicodes/utils'
import { conversationScopedTransport } from '../conversation/scoped-transport'
import { LocalControlService, type ControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'

const cleanup: Array<() => void> = []
afterEach(() => { for (const close of cleanup.splice(0)) close() })

async function fixture(blobs?: BlobStore, wrap: (control: ControlService) => ControlService = (control) => control) {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  const user = await control.createUser({ username: 'reader', passwordHash: 'unused', role: 'user' })
  const conversation = await control.createConversation(user!.id)
  const pair = createInProcessTransportPair()
  const scoped = conversationScopedTransport(pair.server, conversation, {
    control: wrap(control),
    blobs,
    providerAllowed: async (providerId) => providerId !== 'someone-elses',
  })
  const received: ClientFrame[] = []
  const replies: ServerFrame[] = []
  scoped.onFrame((frame) => received.push(frame))
  pair.client.onFrame((frame) => replies.push(frame))
  cleanup.push(() => { scoped.close(); pair.client.close(); db.close() })
  return { control, conversation, scoped, client: pair.client, received, replies, user: user! }
}

test('malformed content and provider frames fail before mutation; later frames still arrive', async () => {
  const f = await fixture()
  for (const frame of [
    { type: 'send', messageId: 'bad', content: null },
    { type: 'open', sessionId: 'bad', cwd: '/ignored' },
    { type: 'send', messageId: 'bad', content: [null] },
    { type: 'send', content: [{ type: 'text', text: 'must not become the title' }] },
  ]) f.client.send(frame as never)
  f.client.send({ type: 'abort' })
  await waitFor(() => f.received.length === 1 && f.replies.length === 4)
  expect(f.received).toEqual([{ type: 'abort' }])
  expect(f.replies.every((frame) => frame.type === 'error' && frame.code === 'invalid_frame')).toBe(true)
  expect((await f.control.getConversation(f.conversation.id))?.title).toBe('New conversation')
})

test('open and set_provider record only a provider the user may name', async () => {
  const f = await fixture()
  const selection = (providerId: string) => ({
    providerId,
    model: { providerId, model: { id: 'm', name: 'm', contextWindow: 1, inputLimit: null, thinking: [], acceptedExtensions: [] }, thinking: null },
  })
  f.client.send({ type: 'open', sessionId: 'ignored', cwd: '/ignored', provider: selection('someone-elses') } as never)
  f.client.send({ type: 'set_provider', provider: selection('someone-elses') } as never)
  f.client.send({ type: 'set_provider', provider: selection('mine') } as never)
  await waitFor(() => f.received.length === 1 && f.replies.length === 2)
  expect(f.replies.every((frame) => frame.type === 'error' && frame.code === 'provider_not_found')).toBe(true)
  expect(await f.control.getConversation(f.conversation.id)).toMatchObject({ providerId: 'mine', modelId: 'm' })
})

test('a failed storage rewrite does not poison subsequent deliveries', async () => {
  let first = true
  const f = await fixture(undefined, (control) => new Proxy(control, {
    get(target, key) {
      if (key === 'defaultConversationTitle') return async (id: string, title: string) => {
        if (first) { first = false; throw new Error('temporary storage failure') }
        return target.defaultConversationTitle(id, title)
      }
      const value = Reflect.get(target, key)
      return typeof value === 'function' ? value.bind(target) : value
    },
  }))
  f.client.send({ type: 'send', messageId: 'one', content: [{ type: 'text', text: 'one' }] })
  f.client.send({ type: 'send', messageId: 'two', content: [{ type: 'text', text: 'two' }] })
  await waitFor(() => f.received.length === 1 && f.replies.length === 1)
  expect(f.received[0]).toMatchObject({ type: 'send', messageId: 'two' })
  expect(f.replies[0]).toMatchObject({ type: 'error', code: 'frame_delivery_failed' })
  expect((await f.control.getConversation(f.conversation.id))?.title).toBe('two')
})

test('attachment references are validated and resolved for sends and steers', async () => {
  const bytes = new Uint8Array([1, 2, 3])
  const blobs: BlobStore = { put: async () => 'blob', get: async () => bytes }
  const f = await fixture(blobs)
  const attachment = await f.control.createAttachment({ userId: f.user.id, sha256: 'blob', mediaType: 'image/png', sizeBytes: 3 })
  const content = [{ type: 'image', source: { type: 'ref', ref: attachment.id } }]
  f.client.send({ type: 'send', messageId: 'one', content } as never)
  f.client.send({ type: 'steer', steerId: 'two', content } as never)
  await waitFor(() => f.received.length === 2)
  for (const frame of f.received) expect(frame).toMatchObject({ content: [{ type: 'image', source: { type: 'binary', data: bytes } }] })
  expect(f.replies).toEqual([])
})

test('an outbound blob write failure is reported without dropping later frames', async () => {
  const f = await fixture({ put: async () => { throw new Error('blob unavailable') }, get: async () => null })
  f.scoped.send({ type: 'transcript_reset', revision: 1, blocks: [{
    type: 'user', id: 'u', turnId: 'turn', createdAt: new Date().toISOString(), preamble: null,
    model: { providerId: 'test', model: { id: 'test', name: 'Test', contextWindow: 1000, inputLimit: null, thinking: [], acceptedExtensions: [] }, thinking: null },
    content: [{ type: 'image', source: { type: 'binary', data: new Uint8Array([1]), mediaType: 'image/png' } }],
  }] })
  f.scoped.send({ type: 'phase', phase: 'idle' })
  await waitFor(() => f.replies.length === 2)
  expect(f.replies[0]).toMatchObject({ type: 'error', code: 'frame_send_failed' })
  expect(f.replies[1]).toEqual({ type: 'phase', phase: 'idle' })
})
