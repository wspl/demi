import { beforeEach, expect, test } from 'bun:test'
import { createPinia, setActivePinia } from 'pinia'
import { useConversations } from './store'

beforeEach(() => setActivePinia(createPinia()))

function newConversation() {
  const store = useConversations()
  const id = store.create()
  return { store, conversation: store.items.find((c) => c.id === id)! }
}

function finish(store: ReturnType<typeof useConversations>) {
  for (let i = 0; i < 200; i++) store.advance()
}

test('queued turns keep order and do not consume the next unsent draft', () => {
  const { store, conversation } = newConversation()
  conversation.draft = 'First'
  store.send(conversation)
  conversation.draft = 'Second'
  store.send(conversation)
  conversation.draft = 'Still editing'
  finish(store)
  const users = conversation.blocks.filter((b) => b.type === 'user')
  expect(users.map((b) => b.content)).toEqual([
    [{ type: 'text', text: 'First' }],
    [{ type: 'text', text: 'Second' }],
  ])
  expect(conversation.draft).toBe('Still editing')
  expect(conversation.status).toBe('done')
  expect(conversation.queue).toEqual([])
})

test('stop preserves partial output and prevents more streaming until resume', () => {
  const { store, conversation } = newConversation()
  conversation.draft = 'Hello'
  store.send(conversation)
  store.advance()
  store.stop(conversation)
  const blocks = JSON.stringify(conversation.blocks)
  finish(store)
  expect(JSON.stringify(conversation.blocks)).toBe(blocks)
  expect(conversation.status).toBe('aborted')
  store.start(conversation)
  finish(store)
  expect(conversation.status).toBe('done')
})

test('simulated failure is recoverable and does not replay the user message', () => {
  const { store, conversation } = newConversation()
  store.failNext = true
  conversation.draft = 'Try this'
  store.send(conversation)
  store.advance()
  expect(conversation.status).toBe('error')
  expect(conversation.stream).toBeNull()
  store.start(conversation)
  finish(store)
  expect(conversation.status).toBe('done')
  expect(conversation.blocks.filter((b) => b.type === 'user')).toHaveLength(1)
})

test('running conversations refuse archive and target changes', () => {
  const { store, conversation } = newConversation()
  conversation.draft = 'Work'
  store.send(conversation)
  store.archive([conversation.id])
  store.move([conversation.id], 'demi')
  expect(conversation.archived).toBe(false)
  expect(conversation.projectId).toBeNull()
  expect(conversation.stream).not.toBeNull()
  finish(store)
  store.move([conversation.id], 'demi')
  store.archive([conversation.id])
  expect(conversation.projectId).toBe('demi')
  expect(conversation.archived).toBe(true)
  conversation.draft = 'Must remain a draft'
  store.send(conversation)
  expect(conversation.stream).toBeNull()
  expect(conversation.draft).toBe('Must remain a draft')
  store.archive([conversation.id], false)
  expect(conversation.archived).toBe(false)
})

test('file-only input is represented in the transcript', () => {
  const { store, conversation } = newConversation()
  conversation.files.push({ id: 'file', name: 'notes.md', destination: 'workspace' })
  store.send(conversation)
  expect(conversation.title).toBe('notes.md')
  const block = conversation.blocks[0]
  expect(block?.type).toBe('user')
  if (block?.type === 'user')
    expect(block.content).toEqual([{ type: 'text', text: 'Workspace file: notes.md' }])
  expect(conversation.files).toEqual([])
})

test('switching main hosts preserves the departed directory and promotes attachments once', () => {
  const { store, conversation } = newConversation()
  store.move([conversation.id], 'demi')
  store.attachHost(conversation, 'mac')
  expect(conversation.attachedHosts).toEqual([])
  store.attachHost(conversation, 'build')
  store.attachHost(conversation, 'build')
  expect(conversation.attachedHosts).toHaveLength(1)
  expect(conversation.attachedHosts[0]!.cwd).toBe('/home/build')
  store.move([conversation.id], null)
  expect(conversation.attachedHosts.find((host) => host.deviceId === 'mac')?.cwd).toBe(
    '/Users/zan/Projects/demi',
  )
  store.move([conversation.id], 'notes')
  expect(conversation.attachedHosts.map((host) => host.deviceId)).toEqual(['build'])
  store.detachHost(conversation, 'build')
  expect(conversation.attachedHosts).toEqual([])
})

test('hostless attachment names are unique', () => {
  const { store, conversation } = newConversation()
  store.attachHost(conversation, 'mac', '/Users/zan', 'worker')
  store.attachHost(conversation, 'build', '/home/build', 'worker')
  expect(conversation.attachedHosts.map((host) => host.name)).toEqual(['worker', 'worker-2'])
})

test('manual order survives new activity and rejects cross-project reorder', () => {
  const { store } = newConversation()
  const one = store.create('demi')
  const two = store.create('demi')
  store.reorder(one, two)
  expect(store.items.findIndex((item) => item.id === one)).toBeLessThan(
    store.items.findIndex((item) => item.id === two),
  )
  const before = store.items.map((item) => item.id)
  const item = store.items.find((item) => item.id === two)!
  item.draft = 'Keep the order'
  store.send(item)
  finish(store)
  expect(store.items.map((item) => item.id)).toEqual(before)
  store.reorder(one, 'writing')
  expect(store.items.map((item) => item.id)).toEqual(before)
})
