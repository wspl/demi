import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { startTinyjsRunner } from '@demicodes/runner/testing'
import { waitFor } from '@demicodes/utils'
import { login, openBackend, type TestBackend, type WebSession } from './session'

// M12 checkpoint 4: the tenant-isolation matrix — every route that names a
// user's object, exercised by another user (and by an admin) against
// alice's objects: 404 as if absent, lists empty. Then revoke and re-claim.

const PNG_BYTES = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0xff, 0xfe, 0x01])
const json = (body: unknown, method = 'POST'): RequestInit => ({ method, body: JSON.stringify(body), headers: { 'content-type': 'application/json' } })

async function createUser(backend: TestBackend, username: string): Promise<WebSession> {
  const response = await backend.session.fetch('/api/users', json({ username, password: `${username}-pass-1`, role: 'user' }))
  if (response.status !== 201) throw new Error(`create ${username}: HTTP ${response.status}`)
  return login(backend, username, `${username}-pass-1`)
}

async function must<T>(response: Response, status: number): Promise<T> {
  if (response.status !== status) throw new Error(`${response.url}: HTTP ${response.status} ${await response.text()}`)
  return (await response.json().catch(() => null)) as T
}

/** Whether a stream socket to `path` opens for `actor`. */
function streamOpens(actor: WebSession, path: string): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = actor.socket(path)
    socket.addEventListener('open', () => (resolve(true), socket.close()), { once: true })
    socket.addEventListener('error', () => resolve(false), { once: true })
    socket.addEventListener('close', () => resolve(false), { once: true })
  })
}

test("the matrix: another user's objects answer 404 on every route, to users and admins alike; revoke, then re-claim", async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'demi-isolation-'))
  const stateDir = await mkdtemp(join(tmpdir(), 'demi-isolation-state-'))
  const home = await mkdtemp(join(tmpdir(), 'demi-isolation-home-'))
  const backend = await openBackend({ dataDir, port: 0, runner: { pingIntervalMs: 0 } })
  const alice = await createUser(backend, 'alice')
  const bob = await createUser(backend, 'bob')

  // Alice's world: a device, a workspace on it, a conversation with a grant, an attachment.
  const runner = await startTinyjsRunner({ backendUrl: backend.url, stateDir, home, name: 'alice-laptop' })
  await waitFor(() => runner.codes.length > 0, () => runner.log.join('\n'), { timeoutMs: 10_000 })
  const { device } = await must<{ device: { id: string } }>(await alice.fetch('/api/devices/claim', json({ code: runner.codes[0] })), 201)
  const { workspace } = await must<{ workspace: { id: string } }>(await alice.fetch('/api/workspaces', json({ deviceId: device.id, path: home, name: 'proj' })), 201)
  const { conversation } = await must<{ conversation: { id: string } }>(await alice.fetch('/api/conversations', { method: 'POST' }), 201)
  await must(await alice.fetch(`/api/conversations/${conversation.id}/grants`, json({ deviceId: device.id })), 201)
  const { attachment } = await must<{ attachment: { id: string; sha256: string } }>(
    await alice.fetch('/api/attachments', { method: 'POST', body: PNG_BYTES, headers: { 'content-type': 'image/png' } }),
    201,
  )
  expect(await streamOpens(alice, `/api/conversations/${conversation.id}/stream`)).toBe(true)

  // Bob's own objects, to aim at alice's from.
  const { conversation: bobs } = await must<{ conversation: { id: string } }>(await bob.fetch('/api/conversations', { method: 'POST' }), 201)

  const denied: Array<[string, string, unknown?]> = [
    ['GET', `/api/conversations/${conversation.id}/transcript`],
    ['PATCH', `/api/conversations/${conversation.id}`, { title: 'taken' }],
    ['PATCH', `/api/conversations/${conversation.id}`, { archived: true }],
    ['GET', `/api/conversations/${conversation.id}/grants`],
    ['POST', `/api/conversations/${conversation.id}/grants`, { deviceId: device.id }],
    ['DELETE', `/api/conversations/${conversation.id}/grants/${device.id}`],
    ['POST', `/api/conversations/${conversation.id}/workspace-files?name=x.txt`, 'bytes'],
    ['PATCH', `/api/conversations/${bobs.id}`, { workspaceId: workspace.id }],
    ['POST', `/api/conversations/${bobs.id}/grants`, { deviceId: device.id }],
    ['DELETE', `/api/devices/${device.id}`],
    ['GET', `/api/devices/${device.id}/fs?path=${encodeURIComponent(home)}`],
    ['POST', `/api/devices/${device.id}/fs`, { path: join(home, 'made') }],
    ['PATCH', `/api/workspaces/${workspace.id}`, { name: 'taken' }],
    ['DELETE', `/api/workspaces/${workspace.id}`],
    ['POST', '/api/workspaces', { deviceId: device.id, path: home, name: 'squat' }],
  ]
  for (const actor of [bob, backend.session]) {
    for (const [method, path, body] of denied) {
      const init: RequestInit =
        body === undefined
          ? { method }
          : typeof body === 'string'
            ? { method, body, headers: { 'content-type': 'application/octet-stream' } }
            : json(body, method)
      const response = await actor.fetch(path, init)
      expect(response.status, `${actor.user.username} ${method} ${path}`).toBe(404)
    }
    expect(await streamOpens(actor, `/api/conversations/${conversation.id}/stream`), `${actor.user.username} stream`).toBe(false)
    const lists = await Promise.all(['/api/conversations', '/api/devices', '/api/workspaces'].map(async (path) => Object.values(await must<Record<string, unknown[]>>(await actor.fetch(path), 200))[0]))
    expect(lists.map((list) => list?.length ?? 0), `${actor.user.username} lists`).toEqual(actor === bob ? [1, 0, 0] : [0, 0, 0])
  }
  // Alice still has everything.
  expect((await must<{ grants: unknown[] }>(await alice.fetch(`/api/conversations/${conversation.id}/grants`), 200)).grants).toHaveLength(1)
  expect((await must<{ devices: unknown[] }>(await alice.fetch('/api/devices'), 200)).devices).toHaveLength(1)
  void attachment

  // Revoke: refused under a workspace, then the runner is refused for good
  // and the grant is gone; re-pairing the machine is a fresh claim, here by bob.
  const inUse = await alice.fetch(`/api/devices/${device.id}`, { method: 'DELETE' })
  expect(inUse.status).toBe(409)
  expect(await inUse.json()).toMatchObject({ code: 'device_in_use' })
  await must(await alice.fetch(`/api/workspaces/${workspace.id}`, { method: 'DELETE' }), 204)
  await must(await alice.fetch(`/api/devices/${device.id}`, { method: 'DELETE' }), 204)
  expect((await must<{ grants: unknown[] }>(await alice.fetch(`/api/conversations/${conversation.id}/grants`), 200)).grants).toHaveLength(0)
  await waitFor(() => runner.statuses.includes('rejected'), undefined, { timeoutMs: 5_000 })
  await runner.stop()
  const again = await startTinyjsRunner({ backendUrl: backend.url, stateDir: await mkdtemp(join(tmpdir(), 'demi-isolation-state2-')), home, name: 'alice-laptop' })
  await waitFor(() => again.codes.length > 0, () => again.log.join('\n'), { timeoutMs: 10_000 })
  expect((await alice.fetch(`/api/devices/${device.id}`, { method: 'DELETE' })).status).toBe(404)
  const reclaimed = await must<{ device: { id: string } }>(await bob.fetch('/api/devices/claim', json({ code: again.codes[0] })), 201)
  expect(reclaimed.device.id).not.toBe(device.id)
  expect((await must<{ devices: Array<{ id: string }> }>(await bob.fetch('/api/devices'), 200)).devices.map((entry) => entry.id)).toEqual([reclaimed.device.id])
  expect((await must<{ devices: unknown[] }>(await alice.fetch('/api/devices'), 200)).devices).toHaveLength(0)
  expect((await alice.fetch(`/api/devices/${reclaimed.device.id}`, { method: 'DELETE' })).status).toBe(404)

  await again.stop()
  await backend.close()
}, 60_000)
