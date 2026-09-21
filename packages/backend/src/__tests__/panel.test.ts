import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { openBackend } from './session'

const json = (body: unknown, method: string) => ({
  method,
  body: JSON.stringify(body),
  headers: { 'content-type': 'application/json' }
})

test('a work panel is saved whole, read back as saved, and never interpreted', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-panel-'))
  const backend = await openBackend({ dataDir, port: 0 })
  try {
    const id = crypto.randomUUID()
    await backend.session.fetch('/api/conversations', json({ id }, 'POST'))
    const path = `/api/conversations/${id}/panel`
    // A conversation that never saved has the empty panel.
    expect(await (await backend.session.fetch(path)).json()).toEqual({ selection: 'change', tabs: [] })

    const panel = {
      selection: 'tab-1',
      tabs: [
        { id: 'tab-1', kind: 'browser', data: { url: 'about:blank' } },
        { id: 'tab-2', kind: 'a kind the backend never heard of', data: [1, { nested: true }] },
      ],
    }
    expect((await backend.session.fetch(path, json(panel, 'PUT'))).status).toBe(204)
    expect(await (await backend.session.fetch(path)).json()).toEqual(panel)
    // The last save wins.
    expect((await backend.session.fetch(path, json({ selection: 'file', tabs: [] }, 'PUT'))).status).toBe(204)
    expect(await (await backend.session.fetch(path)).json()).toEqual({ selection: 'file', tabs: [] })

    const refusals: Array<[unknown, number, string]> = [
      [{ selection: 'change' }, 400, 'invalid_body'],
      [{ selection: 'change', tabs: [{ id: 'a', kind: 'page', data: null, status: 'loading' }] }, 400, 'invalid_body'],
      [{ selection: 'change', tabs: Array.from({ length: 65 }, (_, index) => ({ id: `t${index}`, kind: 'page', data: null })) }, 400, 'invalid_body'],
      [{ selection: 'change', tabs: [{ id: 'a', kind: 'page', data: 'x'.repeat(70_000) }] }, 413, 'too_large'],
    ]
    for (const [body, status, code] of refusals) {
      const answer = await backend.session.fetch(path, json(body, 'PUT'))
      expect(answer.status).toBe(status)
      expect(await answer.json()).toMatchObject({ code })
    }
    expect(await (await backend.session.fetch(path)).json()).toEqual({ selection: 'file', tabs: [] })

    expect((await backend.session.fetch(`/api/conversations/${crypto.randomUUID()}/panel`)).status).toBe(404)
    // An archived conversation reads its panel and refuses a save.
    await backend.session.fetch(`/api/conversations/${id}`, json({ archived: true }, 'PATCH'))
    expect((await backend.session.fetch(path)).status).toBe(200)
    const archived = await backend.session.fetch(path, json(panel, 'PUT'))
    expect(archived.status).toBe(409)
    expect(await archived.json()).toMatchObject({ code: 'conversation_archived' })
  } finally {
    await backend.close()
  }
}, 30_000)
