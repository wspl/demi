import { chmod, mkdir, mkdtemp, rm, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { strict as assert } from 'node:assert'
import { makeSystemImage, mountSystemOverlay, requireTool, runTool } from '../../gvisor/image-tools'

// The test launches this fixture inside a private Linux mount namespace.
process.umask(0o077)
const directory = await mkdtemp(join(tmpdir(), 'demi-overlay-'))
const base = join(directory, 'base')
const volume = join(directory, 'volume')
const root = join(directory, 'root')
try {
  for (const path of [base, volume, root]) await mkdir(path)
  const image = join(directory, 'system.ext4')
  await makeSystemImage(image, 32 * 1024 ** 2)
  await requireTool('mount', ['-o', 'loop,nodev', image, volume])
  await mountSystemOverlay(base, volume, root)
  assert.equal((await stat(root)).mode & 0o777, 0o755)
  await chmod(root, 0o500)
  await requireTool('umount', [root])
  await mountSystemOverlay(base, volume, root)
  assert.equal((await stat(root)).mode & 0o777, 0o500)
} finally {
  for (const path of [root, volume]) {
    if ((await runTool('mountpoint', ['-q', path])).code === 0) await requireTool('umount', [path])
  }
  await rm(directory, { recursive: true, force: true })
}
