import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Hono } from 'hono'
import type { Host } from '@demicodes/shell'
import type { Block } from '@demicodes/core'
import type { AuthEnv } from '../auth/identity'
import { ChangeStore } from '../storage/change-store'
import { DirChangeObjects } from '../storage/change-objects'
import { conversationRoutes } from '../http/conversations'

const roots: string[] = []
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })))
})

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'demi-changes-'))
  roots.push(root)
  const objects = new DirChangeObjects(root)
  const store = new ChangeStore(objects)
  const host = { fs: { readFile: async (path: string) => {
    if (path === 'missing') {
      throw new Error('missing snapshot')
    }
    return new TextEncoder().encode(path)
  } } } as unknown as Host
  const files = await store.retain('source', 'command', host, [
    { path: '/work/file', kind: 'modified', added: 2, removed: 2,
      edits: [{ original: 'before-A', modified: 'after-A' }, { original: 'before-B', modified: 'after-B' }] },
    { path: '/work/new', kind: 'added', added: 1, removed: 0, edits: [{ modified: 'new' }] },
    { path: '/work/binary', kind: 'modified', added: 0, removed: 0, edits: [{}] },
  ])
  return { root, objects, store, files }
}

test('historical pairs survive cold reads, Fork and a missing side', async () => {
  const { objects, store, files, root } = await fixture()
  const cold = new ChangeStore(new DirChangeObjects(root))
  expect(await cold.read('source', 'command', files, '/work/file', 1)).toEqual({ original: 'before-B', modified: 'after-B' })
  expect(await cold.read('source', 'command', files, '/work/new', 0)).toEqual({ original: '', modified: 'new' })
  expect(await cold.read('source', 'command', files, '/work/binary', 0)).toBeNull()
  const blocks = [{ type: 'tool_call', view: { kind: 'shell', commandId: 'command', files } }] as unknown as Block[]
  await store.fork('source', 'fork', blocks)
  await rm(join(root, 'changes/source'), { recursive: true })
  expect(await cold.read('fork', 'command', files, '/work/file', 0)).toEqual({ original: 'before-A', modified: 'after-A' })
  await objects.put('changes/fork/command/0/0.modified', new TextEncoder().encode('intact'))
  await rm(join(root, 'changes/fork/command/0/0.original'))
  expect(await cold.read('fork', 'command', files, '/work/file', 0)).toBeNull()
})

test('history route authorizes the owner and reads archived conversations without a Host', async () => {
  const { store, files } = await fixture()
  const options = {
    changes: store,
    control: { getConversation: async (id: string) => id === 'source' ? { id, userId: 'owner', archived: true } : null },
    conversationStores: { commandFiles: (_id: string, command: string) => command === 'command' ? files : null },
    withHost: () => { throw new Error('History must not reach a Host') },
  } as unknown as Parameters<typeof conversationRoutes>[0]
  const app = new Hono<AuthEnv>()
  app.use('*', async (c, next) => {
    c.set('user', { id: c.req.header('test-user') ?? 'owner', email: '', nickname: '', role: 'user', createdAt: '' })
    await next()
  })
  app.route('/conversations', conversationRoutes(options))
  const path = '/conversations/source/commands/command/changes/file?path=%2Fwork%2Ffile&edit=1'
  expect(await (await app.request(path)).json()).toEqual({ original: 'before-B', modified: 'after-B' })
  expect((await app.request(path, { headers: { 'test-user': 'stranger' } })).status).toBe(404)
  expect((await app.request(path.replace('edit=1', 'edit=99'))).status).toBe(404)
  expect((await app.request(path.replace('edit=1', 'edit=-1'))).status).toBe(400)
  expect((await app.request(path.replace('%2Fwork%2Ffile', '%2Foutside'))).status).toBe(404)
})
