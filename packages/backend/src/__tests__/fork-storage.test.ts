import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { AgentServer, AgentSession } from '@demicodes/agent'
import { StubProvider, events } from '@demicodes/provider/testing'
import { model, text } from '../../../agent/src/__tests__/helpers'
import { createRootRecord } from '../../../agent/src/node/root-record'
import { ConversationForks } from '../conversation/fork'
import { openSqliteDatabase } from '../storage/database'
import { LocalControlService } from '../storage/control'
import { ConversationStores } from '../storage/conversation-store'
import { UserBlobStores } from '../storage/user-blobs'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'demi-fork-storage-'))
  const db = openSqliteDatabase(join(root, 'control.sqlite'))
  migrate(db, CONTROL_MIGRATIONS)
  let failPublication = false
  const control = new LocalControlService({
    ...db,
    run(sql, params) {
      db.run(sql, params)
      if (failPublication && sql.startsWith('INSERT INTO conversations')) {
        throw new Error('publication failed')
      }
    },
  })
  const user = (await control.createMaster({ email: 'fork@example.test', passwordHash: '' }))!
  const source = await control.createConversation(user.id, { title: 'Source' })
  const blobs = new UserBlobStores(join(root, 'blobs'), control)
  const stores = new ConversationStores(join(root, 'conversations'), (id) => blobs.forConversation(id))
  const harness = {
    name: 'fork-test', initialState: () => ({}), restoreState: () => ({}), systemPrompt: () => '',
    host: () => { throw new Error('Fork must not construct a Host') },
  }
  const server = new AgentServer({
    agent: harness, providers: [], store: (id) => stores.treeStore(id),
    shellEnvironment: () => { throw new Error('Fork must not construct a shell') },
  })
  await stores.treeStore(source.id).createNode(createRootRecord(source.id), {
    changedBlocks: [], blockCount: 0, state: {}, phase: 'idle', queue: [], model,
    cwd: '/workspace', harnessName: harness.name,
  })
  const session = new AgentSession({
    provider: new StubProvider([[events.text('answer'), events.response()]]), model, cwd: '/workspace',
    runtime: { harnessName: harness.name, initialState: () => ({}), restoreState: () => ({}), systemPrompt: () => '', tools: () => [] },
  }, { agentSessionId: source.id, store: stores.treeStore(source.id).sessionStore(source.id) })
  await session.commandStorage().writeJson('todos.json', ['pending'])
  await session.send([
    ...text('source question'),
    { type: 'image', source: { type: 'binary', mediaType: 'image/png', data: new Uint8Array([1, 2, 3]) } },
  ])
  await session.commandStorage().writeJson('todos.json', ['done'])
  const blockId = session.transcript().blocks.find((block) => block.type === 'text')!.id
  const forks = () => new ConversationForks({ control, stores, server, registry: { deviceIdentity: () => null } })
  return {
    root, control, source, user, stores, session, blockId, server, forks,
    failPublication: (value: boolean) => { failPublication = value },
    async close() {
      await session.dispose()
      await server.close()
      stores.close()
      db.close()
      await rm(root, { recursive: true, force: true })
    },
  }
}

test('publication failure leaves a hidden complete root that recovery publishes exactly once', async () => {
  const f = await fixture()
  try {
    const id = crypto.randomUUID()
    f.failPublication(true)
    await expect(f.forks().create(f.user.id, f.source.id, id, f.blockId)).rejects.toThrow('publication failed')
    expect(await f.control.getConversation(id)).toBeNull()
    expect((await f.control.listConversations(f.user.id)).map((item) => item.id)).toEqual([f.source.id])
    const committed = await f.stores.treeStore(id).sessionStore(id).load()
    expect(committed?.phase).toBe('idle')
    expect(committed?.queue).toEqual([])
    expect(committed?.commandState.revision).toBe(1)
    await expect(f.control.createConversation(f.user.id, { id })).rejects.toThrow('reserved')
    f.failPublication(false)
    await f.forks().recover()
    await f.forks().recover()
    expect((await f.control.getConversation(id))!.title).toBe('Source (Fork)')
    expect((await f.control.listConversations(f.user.id))).toHaveLength(2)
    const retry = await f.forks().create(f.user.id, f.source.id, id, f.blockId)
    expect(retry.created).toBe(false)
    expect(await f.stores.treeStore(id).sessionStore(id).load()).toEqual(committed)
  } finally {
    await f.close()
  }
})

test('Fork remains independent after source removal and carries no child records', async () => {
  const f = await fixture()
  try {
    const sourceStore = f.stores.treeStore(f.source.id)
    const sourceCheckpoint = (await sourceStore.sessionStore(f.source.id).load())!
    await sourceStore.createNode({ ...createRootRecord('child'), parentId: f.source.id }, {
      ...sourceCheckpoint, changedBlocks: [], blockCount: 0,
      commandState: undefined,
    })
    const id = crypto.randomUUID()
    await f.forks().create(f.user.id, f.source.id, id, f.blockId)
    const destination = f.stores.treeStore(id)
    const before = await destination.sessionStore(id).load()
    expect(await destination.children(id)).toEqual([])
    await f.session.dispose()
    await sourceStore.deleteNode(f.source.id)
    f.stores.db(f.source.id).close()
    await rm(join(f.root, 'conversations', `${f.source.id}.sqlite`))
    expect(await destination.sessionStore(id).load()).toEqual(before)
    expect(before?.transcript.blocks[0]).toMatchObject({
      content: [
        { type: 'text', text: 'source question' },
        { type: 'image', source: { data: new Uint8Array([1, 2, 3]) } },
      ],
    })
  } finally {
    await f.close()
  }
})

test('a reservation without a committed root stays hidden through recovery', async () => {
  const f = await fixture()
  try {
    const id = crypto.randomUUID()
    await f.control.reserveConversationFork({
      id, userId: f.user.id, sourceId: f.source.id, blockId: f.blockId,
      title: 'Reserved (Fork)', model, target: { kind: 'cloud' },
      createdAt: new Date().toISOString(), attachedHosts: [],
    })
    await f.forks().recover()
    expect(await f.control.getConversation(id)).toBeNull()
    const resumed = await f.forks().create(f.user.id, f.source.id, id, f.blockId)
    expect(resumed.conversation.title).toBe('Reserved (Fork)')
    expect(await f.stores.treeStore(id).sessionStore(id).load()).not.toBeNull()
  } finally {
    await f.close()
  }
})
