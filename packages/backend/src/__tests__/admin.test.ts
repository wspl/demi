import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { login, openBackend, type WebSession } from './session'

// M12 checkpoint 2: the admin surface — accounts by role, password resets
// down the ranks, the read-only instance mode.

const json = (body: unknown, method = 'POST'): RequestInit => ({ method, body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })

async function createUser(actor: WebSession, username: string, role: 'admin' | 'user', password = `${username}-pass-1`) {
  const response = await actor.fetch('/api/users', json({ username, password, role }))
  return { status: response.status, body: (await response.json().catch(() => null)) as { user?: { id: string; role: string }; code?: string } }
}

test('accounts: master creates admins and users, admins create users only, nobody outranks the master', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-admin-'))
  const backend = await openBackend({ dataDir, port: 0 })
  const master = backend.session

  const admin = await createUser(master, 'alice', 'admin')
  expect(admin.status).toBe(201)
  expect(admin.body.user).toMatchObject({ role: 'admin' })
  const user = await createUser(master, 'bob', 'user')
  expect(user.status).toBe(201)
  expect((await createUser(master, 'bob', 'user')).body.code).toBe('username_taken')
  expect((await master.fetch('/api/users', json({ username: 'x', password: 'short', role: 'user' }))).status).toBe(400)

  const alice = await login(backend, 'alice', 'alice-pass-1')
  const bob = await login(backend, 'bob', 'bob-pass-1')

  // Listing: admins see every account; a user sees nothing of it.
  const listed = (await (await alice.fetch('/api/users')).json()) as { users: Array<{ username: string; role: string }> }
  expect(listed.users.map((entry) => `${entry.username}:${entry.role}`)).toEqual(['master:master', 'alice:admin', 'bob:user'])
  expect((await bob.fetch('/api/users')).status).toBe(403)
  expect((await createUser(bob, 'carol', 'user')).status).toBe(403)

  // An admin creates users, not admins.
  expect((await createUser(alice, 'carol', 'user')).status).toBe(201)
  expect((await createUser(alice, 'dave', 'admin')).body.code).toBe('forbidden')

  // Resets go down the ranks: admin → user, master → admin, never up or sideways.
  const bobId = user.body.user!.id
  expect((await alice.fetch(`/api/users/${bobId}`, json({ password: 'bob-pass-2' }, 'PATCH'))).status).toBe(204)
  expect((await login(backend, 'bob', 'bob-pass-2')).user.id).toBe(bobId)
  expect((await alice.fetch(`/api/users/${admin.body.user!.id}`, json({ password: 'alice-pass-2' }, 'PATCH'))).status).toBe(403)
  expect((await alice.fetch(`/api/users/${master.user.id}`, json({ password: 'master-pass-2' }, 'PATCH'))).status).toBe(403)
  expect((await master.fetch(`/api/users/${admin.body.user!.id}`, json({ password: 'alice-pass-2' }, 'PATCH'))).status).toBe(204)
  expect((await login(backend, 'alice', 'alice-pass-2')).user.role).toBe('admin')
  expect((await master.fetch('/api/users/nobody', json({ password: 'whatever-1' }, 'PATCH'))).status).toBe(404)

  await backend.close()
})

test('the instance mode is read back as configured', async () => {
  for (const mode of ['shared', 'isolated'] as const) {
    const dataDir = await mkdtemp(join(tmpdir(), `demi-settings-${mode}-`))
    const backend = await openBackend({ dataDir, port: 0, mode })
    expect(await (await backend.session.fetch('/api/settings')).json()).toEqual({ mode })
    expect((await fetch(`${backend.url}/api/settings`)).status).toBe(401)
    await backend.close()
  }
})
