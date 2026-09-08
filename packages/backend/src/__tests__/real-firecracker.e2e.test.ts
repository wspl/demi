import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { delay } from '@demicodes/utils'
import { FirecrackerProvisioner, firecrackerConfigFromEnv } from '../managed/firecracker'
import type { ManagedOperation } from '../storage/control'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'

// Real Linux/KVM and guest filesystem verification, with a scripted provider.
const e2e = process.env.DEMI_FIRECRACKER_E2E === '1' ? test : test.skip

e2e('managed runner: one user machine, matching file/job identity, both disks persist, external reset retains home', async () => {
  const dataDir = await mkdtemp(join(process.env.DEMI_FIRECRACKER_E2E_DATA ?? tmpdir(), 'demi-fc-'))
  const config = firecrackerConfigFromEnv(process.env, dataDir)
  if (!config) throw new Error('DEMI_MANAGED_FIRECRACKER is required')
  const provisioner = new FirecrackerProvisioner(config)
  const publicUrl = process.env.DEMI_FIRECRACKER_E2E_PUBLIC ?? 'http://172.16.0.1:3277'
  const world = await World.create({ dataDir, port: Number(new URL(publicUrl).port), publicUrl, managedHosts: { provisioner, config: { idleMs: 60_000, sweepMs: 60_000 } } })
  try {
    const driver = await world.conversation('cloud')
    await driver.upload('uploaded', new TextEncoder().encode('from-api'))
    const initial = await driver.turn({ model: [model.shell('initial', 'id -u; stat -c %u uploaded; echo home-data > note; sudo sh -c "echo installed > /usr/local/system-marker"; cat uploaded'), model.say('ready')] })
    expect(initial.received[0]).toContain('1000\n1000')
    expect(initial.received[0]).toContain('from-api')
    const status = await world.api<{ device: { id: string } }>('/api/cloud')
    const second = await world.api<{ workspace: { deviceId: string; path: string } }>('/api/workspaces', { cloud: true, name: 'other-project' })
    expect(second.workspace.deviceId).toBe(status.device.id)
    await world.backend.managedHosts!.hibernate(status.device.id)
    const woken = await driver.turn({ model: [model.shell('wake', 'cat note /usr/local/system-marker; ls /run/demi; echo latest-home > note; sudo chmod 000 /usr/bin/bash'), model.say('awake')] })
    expect(woken.received[0]).toContain('home-data\ninstalled')
    const operationId = crypto.randomUUID()
    await world.api('/api/cloud/reset', { operationId })
    const deadline = Date.now() + 60_000
    while ((await world.api<{ operation: ManagedOperation }>('/api/cloud')).operation.phase !== 'ready') {
      if (Date.now() > deadline) throw new Error('Cloud reset did not finish')
      await delay(50)
    }
    const reset = await driver.turn({ model: [model.shell('reset', 'cat note; test ! -e /usr/local/system-marker && echo clean-system'), model.say('reset')] })
    expect(reset.received[0]).toContain('latest-home')
    expect(reset.received[0]).toContain('clean-system')
    expect((await world.api<{ device: { id: string } }>('/api/cloud')).device.id).toBe(status.device.id)
  } finally { await world.close() }
}, 240_000)
