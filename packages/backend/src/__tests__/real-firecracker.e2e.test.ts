import { mkdtemp, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, beforeAll, expect, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { FirecrackerProvisioner, firecrackerConfigFromEnv } from '../managed/firecracker'
import { ownerKey } from '../managed/provisioner'
import { LocalControlService, type ControlService, type ManagedHostOwner } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { itemsText } from './scenarios/model'
import { World } from './scenarios/world'
import { model } from './scenarios/driver'

// The Firecracker smoke (`managed-hosts.md` § Verification): real guests on
// a Linux machine with /dev/kvm, after the install script. Gated:
//
//   DEMI_FIRECRACKER_E2E=1 DEMI_MANAGED_FIRECRACKER=… DEMI_MANAGED_KERNEL=… DEMI_MANAGED_ROOTFS=… \
//   DEMI_FIRECRACKER_E2E_PUBLIC=http://172.16.0.1:3277 bun test packages/backend/src/__tests__/real-firecracker.e2e.test.ts
//
// Records cold-provision and wake latency in its output.

const e2e = process.env.DEMI_FIRECRACKER_E2E === '1' ? test : test.skip

/** The last byte count in a tool result: what `df -B1 --output=size` printed. */
function lastNumber(text: string): number {
  const numbers = text.match(/\d{6,}/g)
  return numbers ? Number(numbers[numbers.length - 1]) : 0
}
const PUBLIC_URL = process.env.DEMI_FIRECRACKER_E2E_PUBLIC ?? 'http://172.16.0.1:3277'
const IDLE_MS = 3_000

let world: World
let provisioner: FirecrackerProvisioner
let control: ControlService
let controlDb: ReturnType<typeof openSqliteDatabase>
const timings: string[] = []

beforeAll(async () => {
  if (process.env.DEMI_FIRECRACKER_E2E !== '1') return
  const dataDir = await mkdtemp(join(process.env.DEMI_FIRECRACKER_E2E_DATA ?? tmpdir(), 'demi-fc-'))
  const config = firecrackerConfigFromEnv({ ...process.env, DEMI_MANAGED_HOME_MIB: process.env.DEMI_MANAGED_HOME_MIB ?? '64', DEMI_MANAGED_SLOTS: process.env.DEMI_MANAGED_SLOTS ?? '4' }, dataDir)
  if (!config) throw new Error('DEMI_MANAGED_FIRECRACKER is required')
  provisioner = new FirecrackerProvisioner(config, { log: (line) => console.log(`[fc] ${line}`) })
  world = await World.create({
    dataDir,
    port: Number(new URL(PUBLIC_URL).port),
    publicUrl: PUBLIC_URL,
    pingIntervalMs: 500,
    managedHosts: {
      provisioner,
      config: { idleMs: IDLE_MS, hardCapMs: 60_000, checkpointIntervalMs: 60_000, sweepMs: 200, hostsPerUser: 2, bootTimeoutMs: 30_000 },
    },
  })
  controlDb = openSqliteDatabase(join(world.dataDir, 'control.sqlite'))
  control = new LocalControlService(controlDb)
})

afterAll(async () => {
  if (process.env.DEMI_FIRECRACKER_E2E !== '1') return
  try {
    await world.close()
  } finally {
    await provisioner.close()
    controlDb.close()
    console.log(timings.join('\n'))
  }
})

e2e('a hostless conversation upgrades to a real guest, works there, hibernates, wakes, grows its home, is destroyed', async () => {
  const driver = await world.conversation('hostless')
  const owner: ManagedHostOwner = { kind: 'conversation', id: driver.id }
  await driver.turn({ model: [model.shell('t0', "mkdir -p work && printf 'alpha\\n' > work/a.txt && echo ok"), model.say('ready')] })

  // Cold provision: the first outside script makes the image, boots the guest, and runs there.
  const coldStart = Date.now()
  const upgraded = await driver.turn({ model: [model.shell('t1', 'echo "$(id -un) in $(pwd) on $(uname -m)"; cat work/a.txt; ls -ld /home/demi/work; demi host current'), model.say('moved')] })
  timings.push(`cold provision + first job: ${Date.now() - coldStart} ms`)
  expect(upgraded.received[0]).toContain('demi in /home/demi on aarch64')
  expect(upgraded.received[0]).toContain('alpha')
  expect(upgraded.received[0]).toContain('demi demi')
  expect(upgraded.received[0]).toContain('host: machine "cloud"')
  expect(itemsText(upgraded.requests[0]!.items)).not.toContain('[Execution target switched]')
  expect(provisioner.running(owner)).toBe(true)

  // The upper: a system-level write works and is the VM's; sudo works; the rootfs stays read-only underneath.
  const upper = await driver.turn({ model: [model.shell('t2', 'sudo touch /usr/local/marker && ls /usr/local/marker && mount | grep -c " / type overlay"'), model.say('upper')] })
  expect(upper.received[0]).toContain('/usr/local/marker')
  expect(upper.received[0]).toContain('1')

  // Idle: the sync, the kill, the shrink, the store.
  await waitFor(() => !provisioner.running(owner), undefined, { timeoutMs: 20_000 })
  const stored = join(world.dataDir, 'homes', `${ownerKey(owner)}.ext4`)
  const shrunk = (await stat(stored)).size
  expect(shrunk).toBeLessThan(64 * 1024 * 1024)
  timings.push(`stored home after hibernate: ${shrunk} bytes`)

  // Wake: the next turn boots a fresh guest over the same home; the upper is gone, the home is there, the fs grew back.
  const wakeStart = Date.now()
  const woken = await driver.turn({ model: [model.shell('t3', 'cat work/a.txt; ls /usr/local/marker 2>&1; df -B1 --output=size /home | tail -1'), model.say('woken')] })
  timings.push(`wake + job: ${Date.now() - wakeStart} ms`)
  expect(woken.received[0]).toContain('alpha')
  expect(woken.received[0]).toContain('cannot access')
  const fsSize = lastNumber(woken.received[0]!)
  expect(fsSize).toBeGreaterThan(50 * 1024 * 1024)

  // Growth: fill past the reserve; the runner asks, the backend enlarges, the guest resizes.
  const grown = await driver.turn({ model: [model.shell('t4', 'dd if=/dev/zero of=fill bs=1M count=50 status=none && sync && echo filled'), model.say('filled')] })
  expect(grown.received[0]).toContain('filled')
  const working = join(world.dataDir, 'firecracker', 'homes', `${ownerKey(owner)}.ext4`)
  const growDeadline = Date.now() + 30_000
  while ((await stat(working)).size <= 64 * 1024 * 1024) {
    if (Date.now() > growDeadline) throw new Error('the home file was not enlarged')
    await new Promise((resolve) => setTimeout(resolve, 200))
  }
  const resized = await driver.turn({ model: [model.shell('t5', 'sleep 2; df -B1 --output=size /home | tail -1'), model.say('resized')] })
  const grownSize = lastNumber(resized.received[0]!)
  timings.push(`home filesystem after growth: ${grownSize} bytes`)
  expect(grownSize).toBeGreaterThan(100 * 1024 * 1024)

  // Archive: the guest is destroyed; the store keeps the image.
  await world.api(`/api/conversations/${driver.id}`, { archived: true }, 'PATCH')
  expect(provisioner.running(owner)).toBe(false)
  expect(await stat(stored).then(() => true, () => false)).toBe(true)
  expect(await control.getManagedDevice(owner)).not.toBeNull()
}, 240_000)
