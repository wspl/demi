import { expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { CONVERSATION_IDLE_MS } from '../../lifecycle/coordinator'
import { model } from './driver'
import { FakeProvisioner } from '../../testing/fake-provisioner'
import { World } from './world'

test('paired release uses one hour, resets on activity, and reaches main and attached devices', async () => {
  let now = 0
  const world = await World.create({ runners: ['main', 'attached'], lifecycle: { now: () => now, pollMs: 10 } })
  try {
    const driver = await world.conversation('runner:main')
    await world.api(`/api/conversations/${driver.id}/hosts`, { deviceId: world.device('attached').deviceId })
    const endpoint = `/api/conversations/${driver.id}/fs`
    await world.api(endpoint)
    await Bun.sleep(30)
    now = CONVERSATION_IDLE_MS - 1
    await world.api(endpoint)
    await Bun.sleep(30)
    now = CONVERSATION_IDLE_MS
    await Bun.sleep(30)
    expect(world.frames.some(frame => frame.message.type === 'conversation_release')).toBe(false)
    now = 2 * CONVERSATION_IDLE_MS - 1
    await waitFor(() => world.frames.filter(frame => frame.message.type === 'conversation_released').length === 2)
    expect(world.frames.filter(frame => frame.message.type === 'conversation_release').map(frame => frame.deviceId).sort())
      .toEqual([world.device('main').deviceId, world.device('attached').deviceId].sort())
    const turn = await driver.turn({ model: [model.shell('still-online', 'printf available', 10_000), model.say('done')] })
    expect(turn.received[0]).toContain('available')
  } finally {
    await world.close()
  }
}, 45_000)

test('a target switch invalidates the old idle deadline', async () => {
  let now = 0
  const world = await World.create({ runners: ['old', 'new'], lifecycle: { now: () => now, pollMs: 10 } })
  try {
    const driver = await world.conversation('runner:old')
    await world.api(`/api/conversations/${driver.id}/fs`)
    await Bun.sleep(30)
    now = CONVERSATION_IDLE_MS - 1
    await driver.switchTo('runner:new')
    await Bun.sleep(30)
    now++
    await Bun.sleep(30)
    const releases = world.frames.filter(frame => frame.message.type === 'conversation_release')
    expect(releases).toHaveLength(1)
    expect(releases[0]?.deviceId).toBe(world.device('old').deviceId)
  } finally {
    await world.close()
  }
}, 45_000)

test('paired Host activity resets the idle deadline of an attached Cloud', async () => {
  let now = 0
  const fake = new FakeProvisioner()
  const world = await World.create({ runners: ['paired'], lifecycle: { now: () => now, pollMs: 10 }, managedHosts: { provisioner: fake, config: { sweepMs: 10 } } })
  try {
    const driver = await world.conversation('cloud')
    const endpoint = `/api/conversations/${driver.id}/fs`
    await world.api(endpoint)
    await driver.switchTo('runner:paired')
    await Bun.sleep(30)
    now = CONVERSATION_IDLE_MS - 1
    await world.api(endpoint)
    await Bun.sleep(30)
    now++
    await Bun.sleep(30)
    expect(fake.calls.some(call => call.startsWith('hibernate:'))).toBe(false)
    now = 2 * CONVERSATION_IDLE_MS - 1
    await waitFor(() => fake.calls.some(call => call.startsWith('hibernate:')))
  } finally {
    await world.close()
  }
}, 45_000)

test('target switch, attachment detach and archive release before changing the binding', async () => {
  const world = await World.create({ runners: ['one', 'two'] })
  try {
    const { conversation: driver } = await world.api<{ conversation: { id: string } }>('/api/conversations', { id: crypto.randomUUID() })
    await world.api(`/api/conversations/${driver.id}`, { target: { kind: 'workspace', workspaceId: world.device('one').workspaceId } }, 'PATCH')
    await world.api(`/api/conversations/${driver.id}`, { target: { kind: 'workspace', workspaceId: world.device('two').workspaceId } }, 'PATCH')
    expect(world.frames.filter(frame => frame.message.type === 'conversation_released')).toHaveLength(1)
    expect(world.frames.find(frame => frame.message.type === 'conversation_release')?.deviceId).toBe(world.device('one').deviceId)
    await world.api(`/api/conversations/${driver.id}/hosts/${world.device('one').deviceId}`, undefined, 'DELETE')
    expect(world.frames.filter(frame => frame.message.type === 'conversation_released')).toHaveLength(2)
    await world.api(`/api/conversations/${driver.id}`, { archived: true }, 'PATCH')
    expect(world.frames.filter(frame => frame.message.type === 'conversation_released')).toHaveLength(3)
    expect(world.frames.filter(frame => frame.message.type === 'conversation_release').at(-1)?.deviceId).toBe(world.device('two').deviceId)
  } finally {
    await world.close()
  }
}, 45_000)

test('one hour of Cloud inactivity stops the machine without conversation release', async () => {
  let now = 0
  const fake = new FakeProvisioner()
  const world = await World.create({ lifecycle: { now: () => now, pollMs: 10 }, managedHosts: { provisioner: fake, config: { sweepMs: 10 } } })
  try {
    const driver = await world.conversation('cloud')
    await world.api(`/api/conversations/${driver.id}/fs`)
    await Bun.sleep(30)
    now = CONVERSATION_IDLE_MS - 1
    await Bun.sleep(30)
    expect(fake.calls.some(call => call.startsWith('hibernate:'))).toBe(false)
    now++
    await waitFor(() => fake.calls.some(call => call.startsWith('hibernate:')))
    expect(world.frames.some(frame => frame.message.type === 'conversation_release')).toBe(false)
  } finally {
    await world.close()
  }
}, 45_000)
