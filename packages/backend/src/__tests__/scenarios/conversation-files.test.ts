import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { FakeProvisioner } from './fake-provisioner'
import { World } from './world'

test('conversation files boot Cloud, wake it after idle, and follow a target switch', async () => {
  const fake = new FakeProvisioner()
  const world = await World.create({
    runners: ['paired'],
    managedHosts: {
      provisioner: fake,
      config: {
        idleMs: 500,
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
