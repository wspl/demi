import { expect, test } from 'bun:test'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { LocalHost } from '../node'
import { hostConformanceCases } from '../testing'

const root = await mkdtemp(join(tmpdir(), 'demi-local-host-'))
const host = new LocalHost(root, { storeRoot: join(root, 'store'), commandArtifactsDir: join(root, 'artifacts') })

for (const conformance of hostConformanceCases({ host, root })) {
  test(`LocalHost ${conformance.name}`, conformance.run)
}

test('LocalHost spawn after unlinking cwd follows the platform cwd anchor', async () => {
  const dir = join(root, 'gone')
  await mkdir(dir)
  const cwd = await host.process.openCwd(dir)
  await rm(dir, { recursive: true, force: true })
  const handle = await host.process.spawn({
    command: '/bin/echo',
    args: ['ok'],
    cwd: cwd.spawnPath(),
    env: { PATH: '/usr/bin:/bin' },
  })
  const [stdout, exit] = await Promise.all([collectText(handle.stdout), handle.wait()])
  if (process.platform === 'linux') {
    // The /proc/self/fd anchor keeps the unlinked directory reachable.
    expect(exit.exitCode).toBe(0)
    expect(stdout).toBe('ok\n')
  } else {
    // Without a dirfd anchor (macOS devfs cannot traverse directory fds) the
    // deleted path is honestly unusable.
    expect(exit.exitCode).toBeNull()
    expect(exit.spawnError?.kind).toBe('cwd_unusable')
  }
  await cwd.close()
})

async function collectText(iterable: AsyncIterable<Uint8Array>): Promise<string> {
  const chunks: Uint8Array[] = []
  for await (const chunk of iterable) chunks.push(chunk)
  return Buffer.concat(chunks).toString('utf8')
}
