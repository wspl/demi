import { mkdir, mkdtemp, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import {
  growImage,
  makeHomeImage,
  missingImageTools,
  runTool
} from '../managed/firecracker/image-tools'

// Ext4 tools run only when installed; this test never boots a model.
const tools = missingImageTools().length === 0 ? test : test.skip

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

  await growImage(image, nominal * 2)
  expect((await stat(image)).size).toBe(nominal * 2)
  // Growing the backing file leaves the filesystem to the guest; the image is still consistent.
  const check = await runTool('e2fsck', ['-fn', image])
  expect(check.code).toBe(0)
}, 60_000)
