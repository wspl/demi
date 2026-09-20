import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { FakeProvisioner } from '../../testing/fake-provisioner'
import { World } from './world'

test('conversation files boot Cloud, wake it after idle, and follow a target switch', async () => {
  const fake = new FakeProvisioner()
  const world = await World.create({
    runners: ['paired'],
    lifecycle: { idleMs: 500 },
    managedHosts: {
      provisioner: fake,
      config: {
        sweepMs: 50,
        checkpointIntervalMs: 60_000,
        bootTimeoutMs: 15_000,
      },
    },
  })
  try {
    const conversation = await world.conversation('cloud')
    const endpoint = `/api/conversations/${conversation.id}/fs`
    expect(fake.calls).toEqual([])
    const directory = await world.api<{ path: string; home: string; entries: unknown[] }>(endpoint)
    expect(directory.path).toBe(join(directory.home, 'sessions', conversation.id))
    expect(directory.entries).toEqual([])
    const owner = [...fake.guests.keys()][0]!
    const path = join(directory.path, 'note.txt')
    await writeFile(path, 'Cloud file\n')
    await world.api(endpoint, { path: join(directory.path, 'nested', 'directory') })
    const listed = await world.api<{ entries: Array<{ name: string; isDirectory: boolean }> }>(endpoint)
    expect(listed.entries).toEqual(expect.arrayContaining([
      expect.objectContaining({ name: 'note.txt', isDirectory: false }),
      expect.objectContaining({ name: 'nested', isDirectory: true }),
    ]))
    await waitFor(
      () => fake.calls.includes(`hibernate:${owner}`) && !fake.running(owner),
      () => fake.calls.join(','),
      { timeoutMs: 10_000 },
    )
    const text = await world.api(`${endpoint}/file?${new URLSearchParams({ path })}`)
    expect(text).toEqual({ path, text: 'Cloud file\n' })
    expect(fake.calls.filter((call) => call === `wake:${owner}`)).toHaveLength(2)

    const missing = await world.backend.session.fetch(`${endpoint}/file?${new URLSearchParams({ path: join(directory.path, 'missing') })}`)
    expect(missing.status).toBe(404)
    const invalid = await world.backend.session.fetch(`${endpoint}/file`)
    expect(invalid.status).toBe(400)
    const unknown = await world.backend.session.fetch(`/api/conversations/${crypto.randomUUID()}/fs`)
    expect(unknown.status).toBe(404)
    await writeFile(join(directory.path, 'binary'), new Uint8Array([0, 255]))
    const binary = await world.backend.session.fetch(`${endpoint}/file?${new URLSearchParams({ path: join(directory.path, 'binary') })}`)
    expect(binary.status).toBe(415)

    const paired = world.device('paired')
    await conversation.switchTo('runner:paired')
    await writeFile(join(paired.home, 'paired.txt'), 'Paired file\n')
    for (let index = 0; index < 40; index += 1) {
      await writeFile(join(paired.home, `entry-${index}.txt`), 'metadata\n')
    }
    const moved = await world.api<{ path: string; entries: Array<{ name: string }> }>(endpoint)
    expect(moved.path).toBe(paired.home)
    expect(moved.entries.filter((entry) => entry.name.startsWith('entry-'))).toHaveLength(40)
    expect(moved.entries.some((entry) => entry.name === 'paired.txt')).toBe(true)
    const pairedText = await world.api(`${endpoint}/file?${new URLSearchParams({ path: join(paired.home, 'paired.txt') })}`)
    expect(pairedText).toEqual({ path: join(paired.home, 'paired.txt'), text: 'Paired file\n' })
    await world.killRunner('paired')
    const offline = await world.backend.session.fetch(endpoint)
    expect(offline.status).toBe(409)
    expect(await offline.json()).toMatchObject({ code: 'device_offline' })

    await world.api(`/api/conversations/${conversation.id}`, { archived: true }, 'PATCH')
    const archived = await world.backend.session.fetch(endpoint)
    expect(archived.status).toBe(409)
    expect(await archived.json()).toMatchObject({ code: 'conversation_archived' })
  } finally {
    await world.close()
  }
}, 45_000)


test('attachment browsing admits only bound hosts and wakes an attached Cloud', async () => {
  const fake = new FakeProvisioner()
  const world = await World.create({
    runners: ['paired', 'unattached'],
    lifecycle: { idleMs: 500 },
    managedHosts: {
      provisioner: fake,
      config: {
        sweepMs: 50,
        checkpointIntervalMs: 60_000,
        bootTimeoutMs: 15_000,
      },
    },
  })
  try {
    const conversation = await world.conversation('cloud')
    const base = `/api/conversations/${conversation.id}`
    const cloudDirectory = await world.api<{ path: string }>(`${base}/fs`)
    const cloud = await world.api<{ device: { id: string } }>('/api/cloud')
    const deviceId = cloud.device.id
    await writeFile(join(cloudDirectory.path, 'cloud-note.txt'), 'preserved')
    const cloudEndpoint = `${base}/hosts/${deviceId}/fs`
    const main = await world.api<{ path: string }>(cloudEndpoint)
    expect(main.path).toBe(cloudDirectory.path)
    const paired = world.device('paired')
    const unbound = await world.backend.session.fetch(`${base}/hosts/${world.device('unattached').deviceId}/fs`)
    expect(unbound.status).toBe(404)
    expect(await unbound.json()).toMatchObject({ code: 'host_not_attached' })
    await conversation.switchTo('runner:paired')
    await waitFor(
      () => fake.calls.includes(`hibernate:${deviceId}`) && !fake.running(deviceId),
      () => fake.calls.join(','),
      { timeoutMs: 10_000 },
    )
    const attached = await world.api<{ path: string; entries: Array<{ name: string }> }>(cloudEndpoint)
    expect(attached.path).toBe(cloudDirectory.path)
    expect(attached.entries.some((entry) => entry.name === 'cloud-note.txt')).toBe(true)
    expect(fake.calls.filter((call) => call === `wake:${deviceId}`)).toHaveLength(2)
    await world.api(cloudEndpoint, { path: join(cloudDirectory.path, 'from-picker') })
    const created = await world.api<{ entries: Array<{ name: string }> }>(cloudEndpoint)
    expect(created.entries.some((entry) => entry.name === 'from-picker')).toBe(true)

    const oldRead = await world.backend.session.fetch(`/api/devices/${paired.deviceId}/fs/file?path=${encodeURIComponent(paired.home)}`)
    expect(oldRead.status).toBe(404)
    const cloudBypass = await world.backend.session.fetch(`/api/devices/${deviceId}/fs`)
    expect(cloudBypass.status).toBe(404)
    await world.api(`${base}/hosts/${deviceId}`, undefined, 'DELETE')
    const detached = await world.backend.session.fetch(cloudEndpoint)
    expect(detached.status).toBe(404)
    expect(await detached.json()).toMatchObject({ code: 'host_not_attached' })
    const current = await world.api<{ path: string }>(`${base}/hosts/${paired.deviceId}/fs`)
    expect(current.path).toBe(paired.home)
    await world.api(base, { archived: true }, 'PATCH')
    const archived = await world.backend.session.fetch(`${base}/hosts/${paired.deviceId}/fs`)
    expect(archived.status).toBe(409)
    expect(await archived.json()).toMatchObject({ code: 'conversation_archived' })
  } finally {
    await world.close()
  }
}, 45_000)
