import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { delay } from '@demicodes/utils'
import {
  FirecrackerProvisioner,
  firecrackerConfigFromEnv
} from '@demicodes/machines'
import type { ManagedOperation } from '../storage/control'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'

// Real Linux/KVM and guest filesystem verification, with a scripted provider.
const e2e = process.env.DEMI_FIRECRACKER_E2E === '1' ? test : test.skip

e2e(
  'managed runner: one user machine, matching file/job identity, both disks persist, external reset retains home',
  async () => {
    const dataDir = await mkdtemp(join(process.env.DEMI_FIRECRACKER_E2E_DATA ??
      tmpdir(), 'demi-fc-'))
    const config = firecrackerConfigFromEnv(process.env, dataDir)
    if (!config)
      throw new Error('DEMI_MANAGED_FIRECRACKER is required')
    const provisioner = new FirecrackerProvisioner(config)
    const publicUrl = process.env.DEMI_FIRECRACKER_E2E_PUBLIC ??
      'http://172.16.0.1:3277'
    const world = await World.create({
      dataDir,
      port: Number(new URL(publicUrl).port),
      publicUrl,
      managedHosts: {
        provisioner,
        config: { idleMs: 60_000, sweepMs: 60_000 }
      }
    })
    try {
      const driver = await world.conversation('cloud')
      const uploaded = await driver.upload('uploaded', new TextEncoder().encode('from-api'))
      const uploadedPath = driver.attachmentPath('uploaded')
      const initial = await driver.turn({
        content: [{ type: 'text', text: 'go' }, uploaded],
        model: [
          model.shell(
            'initial',
            `id -u; stat -c %u ${uploadedPath}; echo home-data > note; sudo sh -c "echo installed > /usr/local/system-marker"; demi file read ${uploadedPath}; demi --help > /dev/null`
          ),
          model.say('ready')
        ]
      })
      expect(initial.received[0]).toContain('1000\n1000')
      expect(initial.received[0]).toContain('from-api')
      const status = await world.api<{ device: { id: string } }>('/api/cloud')
      const second = await world.api<{ workspace: {
          deviceId: string;
          path: string
        } }>('/api/workspaces', { cloud: true, name: 'other-project' })
      expect(second.workspace.deviceId).toBe(status.device.id)
      await world.backend.managedHosts!.hibernate(status.device.id)
      const woken = await driver.turn({ model: [
          model.shell(
            'wake',
            'cat note /usr/local/system-marker; ls /run/demi; echo latest-home > note; sudo chmod 000 /usr/bin/bash'
          ),
          model.say('awake')
        ] })
      expect(woken.received[0]).toContain('home-data\ninstalled')
      const operationId = crypto.randomUUID()
      await world.api('/api/cloud/reset', { operationId })
      const deadline = Date.now() + 60_000
      while ((await world.api<{ operation: ManagedOperation }>('/api/cloud')).operation.phase !== 'ready') {
        if (Date.now() > deadline)
          throw new Error('Cloud reset did not finish')
        await delay(50)
      }
      const reset = await driver.turn({ model: [
          model.shell(
            'reset',
            'cat note; test ! -e /usr/local/system-marker && echo clean-system'
          ),
          model.say('reset')
        ] })
      expect(reset.received[0]).toContain('latest-home')
      expect(reset.received[0]).toContain('clean-system')
      expect(
        (await world.api<{ device: { id: string } }>('/api/cloud')).device.id
      )
        .toBe(status.device.id)
    } finally {
      await world.close()
    }
  },
  240_000
)

// The login shell on the real guest (`runner.md` § Jobs and the tee): a
// toolchain installed by one command is on PATH for the next, with nothing
// sourced — rustup through `~/.profile`, nvm through the skeleton's
// `~/.bashrc`. Needs egress from the guest; several minutes.
e2e(
  'managed runner: rustup in one command and cargo in the next; nvm likewise',
  async () => {
    const dataDir = await mkdtemp(join(process.env.DEMI_FIRECRACKER_E2E_DATA ??
      tmpdir(), 'demi-fc-login-'))
    const config = firecrackerConfigFromEnv(process.env, dataDir)
    if (!config)
      throw new Error('DEMI_MANAGED_FIRECRACKER is required')
    const publicUrl = process.env.DEMI_FIRECRACKER_E2E_PUBLIC ??
      'http://172.16.0.1:3277'
    const world = await World.create({
      dataDir,
      port: Number(new URL(publicUrl).port),
      publicUrl,
      managedHosts: {
        provisioner: new FirecrackerProvisioner(config),
        config: { idleMs: 600_000, sweepMs: 600_000 }
      }
    })
    try {
      const driver = await world.conversation('cloud')
      const facts = await driver.turn({ model: [
          model.shell('facts', 'ls -A ~; uv --version; node --version'),
          model.say('facts')
        ] })
      expect(facts.received[0]).toContain('.profile')
      expect(facts.received[0]).toContain('uv 0.')
      await driver.turn({ model: [
          model.shell(
            'rustup',
            "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal > /dev/null 2>&1; echo rustup-exit=$?",
            600_000
          ),
          model.say('installed')
        ] })
      const cargo = await driver.turn({ model: [
          model.shell('cargo', 'cargo --version'),
          model.say('cargo')
        ] })
      expect(cargo.received[0]).toContain('cargo 1.')
      await driver.turn({ model: [
          model.shell(
            'nvm',
            'curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.7/install.sh | bash > /dev/null 2>&1; . ~/.bashrc; nvm install --lts > /dev/null 2>&1; echo nvm-exit=$?',
            600_000
          ),
          model.say('nvm')
        ] })
      const node = await driver.turn({ model: [
          model.shell('node', 'node --version; command -v node'),
          model.say('node')
        ] })
      expect(node.received[0]).toContain('/.nvm/versions/node/')
    } finally {
      await world.close()
    }
  },
  1_200_000
)
