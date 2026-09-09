import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { login, MASTER, openBackend } from './session'

const json = (body: unknown) => ({ method: 'PATCH', body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })

test('settings patches merge fields across devices, survive restart, and remain private to the user', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-preferences-'))
  let backend = await openBackend({ dataDir, port: 0 })
  try {
    const device = await login(backend, MASTER.email, MASTER.password)
    const path = '/api/settings/preferences'
    const updates = await Promise.all([
      backend.session.fetch(path, json({ appearance: { theme: 'dark' } })),
      device.fetch(path, json({ shortcuts: { new: '⌘⇧N' } })),
      device.fetch(path, json({ appearance: { fontSize: 17 } })),
    ])
    expect(updates.map(response => response.status)).toEqual([200, 200, 200])
    const expected = { preferences: { appearance: { theme: 'dark', fontSize: 17 }, shortcuts: { new: '⌘⇧N' } } }
    expect(await (await device.fetch(path)).json()).toEqual(expected)
    expect((await device.fetch(path, json({ appearance: { fontSize: 100 } }))).status).toBe(400)
    expect((await device.fetch(path, json({ shortcuts: { arbitrary: 'x' } }))).status).toBe(400)
    await backend.close()
    backend = await openBackend({ dataDir, port: 0 })
    expect(await (await backend.session.fetch(path)).json()).toEqual(expected)
    await backend.session.fetch('/api/users', { ...json({ email: 'other@example.test', password: 'other-pass-1', role: 'user' }), method: 'POST' })
    const other = await login(backend, 'other@example.test', 'other-pass-1')
    expect(await (await other.fetch(path)).json()).toEqual({ preferences: { appearance: {}, shortcuts: {} } })
    expect(await (await backend.session.fetch(path, json({ shortcuts: { new: null } }))).json()).toEqual({ preferences: { appearance: expected.preferences.appearance, shortcuts: {} } })
  } finally {
    await backend.close()
  }
})
