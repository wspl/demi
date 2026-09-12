import { createProductClient, type ProductAgentClient } from './session'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { defineProvider } from '@demicodes/provider'
import { events } from '@demicodes/provider/testing'
import { deferred } from '@demicodes/utils'
import { openBackend } from './session'

const json = (body: unknown, method = 'POST') => ({
  method,
  body: JSON.stringify(body),
  headers: { 'content-type': 'application/json' }
})

test(
  'manual ordering, pinning and partial patches persist independently of activity',
  async () => {
    const dataDir = await mkdtemp(join(tmpdir(), 'demi-sidebar-'))
    let backend = await openBackend({ dataDir, port: 0 })
    try {
      const create = async () => (await (await backend.session.fetch(
        '/api/conversations',
        { method: 'POST', body: JSON.stringify({ id: crypto.randomUUID() }) }
      )).json() as { conversation: { id: string } }).conversation.id
      const a = await create()
      const b = await create()
      const c = await create()
      const list = async () => (await (await backend.session.fetch(
        '/api/conversations'
      )).json() as { conversations: Array<{
          id: string;
          title: string
        }> }).conversations
      expect((await list()).map(row => row.id)).toEqual([c, b, a])
      expect(
        (await backend.session.fetch(
          '/api/sidebar/reorder',
          json({ kind: 'conversation', id: a, beforeId: c })
        )).status
      )
        .toBe(204)
      const partial = await backend.session.fetch(
        `/api/conversations/${b}`,
        json(
          {
            title: 'kept title',
            target: { kind: 'workspace', workspaceId: 'missing' }
          },
          'PATCH'
        )
      )
      expect(partial.status).toBe(207)
      expect(await partial.json()).toMatchObject({
        conversation: { title: 'kept title' },
        results: [
          { field: 'title', status: 'applied' },
          { field: 'target', status: 'failed' }
        ]
      })
      expect((await list()).map(row => row.id)).toEqual([a, c, b])
      await backend.session.fetch(
        `/api/conversations/${b}`,
        json({ pinned: true }, 'PATCH')
      )
      expect((await list()).map(row => row.id)).toEqual([b, a, c])
      expect(
        (await backend.session.fetch(
          '/api/sidebar/reorder',
          json({ kind: 'conversation', id: a, beforeId: b })
        )).status
      )
        .toBe(409)
      const batch = await backend.session.fetch(
        '/api/conversations/batch',
        json(
          { items: [
              { id: a, patch: { title: 'renamed' } },
              { id: 'missing', patch: { archived: true } }
            ] }
        )
      )
      expect(batch.status).toBe(207)
      expect(await batch.json()).toMatchObject({ results: [
          { id: a, conversation: { title: 'renamed' } },
          { id: 'missing', code: 'conversation_not_found' }
        ] })
      await backend.close()
      backend = await openBackend({ dataDir, port: 0 })
      expect((await list()).map(row => row.id)).toEqual([b, a, c])
    } finally {
      await backend.close()
    }
  }
)

test(
  'running work refuses archive; completed output is unread until acknowledged; archived sockets refuse writes',
  async () => {
    const entered = deferred<void>()
    const release = deferred<void>()
    let calls = 0
    const backend = await openBackend({
      dataDir: await mkdtemp(join(tmpdir(), 'demi-archive-')),
      port: 0,
      providerTypes: {
        fixture: {
          credential: 'api_key',
          create: ({ providerId, label }) => defineProvider({
            id: providerId,
            displayName: label,
            createRuntime: () => {
              const runtime = {
                async *run() {
                  calls += 1
                  entered.resolve()
                  await release.promise
                  yield events.text('complete')
                  yield events.response()
                },
                clone: () => runtime,
              }
              return runtime
            },
          })
        },
      }
    })
    let client: ProductAgentClient | undefined
    try {
      const { provider } = await (await backend.session.fetch(
        '/api/providers',
        json({
          providerType: 'fixture',
          label: 'Fixture',
          apiKey: 'fake'
        })
      )).json() as { provider: { id: string } }
      const { conversation } = await (await backend.session.fetch(
        '/api/conversations',
        { method: 'POST', body: JSON.stringify({ id: crypto.randomUUID() }) }
      )).json() as { conversation: { id: string } }
      const path = `/api/conversations/${conversation.id}`
      const socket = backend.session.socket(`${path}/stream`)
      await new Promise<void>(
        resolve => socket.addEventListener(
          'open',
          () => resolve(),
          { once: true }
        )
      )
      client = createProductClient(socket)
      await client.open({
        providerId: provider.id,
        model: {
          providerId: provider.id,
          model: {
            id: 'fixture',
            name: 'Fixture',
            contextWindow: 100000,
            outputLimit: null,
            inputLimit: null,
            thinking: [],
            acceptedExtensions: []
          },
          thinking: null
        }
      }, '/ignored', 'ignored')
      const sending = client.send([{ type: 'text', text: 'run' }])
      await entered.promise
      expect(
        (await backend.session.fetch(path, json({ archived: true }, 'PATCH'))).status
      )
        .toBe(409)
      release.resolve()
      await sending
      const { conversations } = await (await backend.session.fetch(
        '/api/conversations'
      )).json() as { conversations: Array<{
          revision: number;
          unread: boolean;
          status: string
        }> }
      expect(conversations[0])
        .toMatchObject({ unread: true, status: 'completed' })
      const revision = conversations[0]!.revision
      expect(
        (await backend.session.fetch(
          `${path}/read`,
          json({ revision: revision + 1 })
        )).status
      )
        .toBe(409)
      expect(
        (await backend.session.fetch(`${path}/read`, json({ revision }))).status
      )
        .toBe(204)
      expect(
        (await backend.session.fetch(path, json({ archived: true }, 'PATCH'))).status
      )
        .toBe(200)
      const refusal = new Promise<void>(resolve => {
        const stop = client!.subscribe(event => {
          if (event.type === 'error' && event.code === 'archived') {
            stop()
            resolve()
          }
        })
      })
      socket.send(JSON.stringify({
        type: 'send',
        messageId: 'forbidden',
        content: [{ type: 'text', text: 'must not run' }]
      }))
      await refusal
      expect(calls).toBe(1)
      expect((await backend.session.fetch(`${path}/transcript`)).status)
        .toBe(200)
      expect(
        (await backend.session.fetch(
          `${path}/workspace-files?name=x`,
          { method: 'POST', body: 'x' }
        )).status
      )
        .toBe(409)
    } finally {
      release.resolve()
      await client?.close()
      await backend.close()
    }
  }
)
