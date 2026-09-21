import { mkdir, mkdtemp, stat, writeFile, truncate, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import {
  recoverVolume,
  makeHomeImage,
  missingImageTools,
  runTool
} from '../gvisor/image-tools'

// Ext4 tools run only when installed; this test never boots a model.
const tools = missingImageTools().length === 0 ? test : test.skip
const privileged = process.platform === 'linux' && process.getuid?.() === 0 &&
  process.env.DEMI_GVISOR_STORAGE_E2E === '1' ? test : test.skip

privileged('a restrictive manager umask permits a new Cloud root and preserves later user modes', async () => {
  const result = await runTool('unshare', ['--mount', '--propagation', 'private',
    process.execPath, '--conditions', 'development',
    new URL('./fixtures/overlay-permissions.ts', import.meta.url).pathname])
  expect(result.stderr).toBe('')
  expect(result.code).toBe(0)
})

tools('make and grow a home image round trip', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-image-'))
  const home = join(dir, 'home')
  await mkdir(join(home, 'work'), { recursive: true })
  await writeFile(join(home, 'work', 'a.txt'), 'alpha\n')
  const image = join(dir, 'home.ext4')
  const nominal = 64 * 1024 * 1024
  await makeHomeImage(home, image, nominal)
  expect((await stat(image)).size).toBe(nominal)
  expect(await stat(home).then(() => true, () => false)).toBe(false)
  const listed = await runTool('debugfs', ['-R', 'cat /demi/work/a.txt', image])
  expect(listed.stdout).toBe('alpha\n')

  await truncate(image, nominal * 2)
  expect(await recoverVolume(image)).toBe(nominal * 2)
  expect((await stat(image)).size).toBe(nominal * 2)
  // Recovery grows the filesystem before publication.
  const check = await runTool('e2fsck', ['-fn', image])
  expect(check.code).toBe(0)
  await rm(dir, { recursive: true })
}, 60_000)
