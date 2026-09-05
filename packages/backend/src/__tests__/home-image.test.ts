import { mkdir, mkdtemp, readFile, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { growImage, makeHomeImage, missingImageTools, parseResizedBlocks, runTool, shrinkImage } from '../managed/firecracker/image-tools'
import { DirHomeImageStore } from '../storage/home-image-store'

// The home-image store and the image tools. The store runs anywhere; the
// tools need e2fsprogs on the machine and skip without them (they run in
// the Linux fixture).

test('the store: put consumes the file and replaces the previous image, get makes a working copy', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-homes-'))
  const store = new DirHomeImageStore(join(dir, 'homes'))
  const owner = 'workspace:demo'
  expect(await store.has(owner)).toBe(false)
  const first = join(dir, 'first.ext4')
  await writeFile(first, 'one')
  await store.put(owner, first)
  expect(await store.has(owner)).toBe(true)
  expect(await stat(first).then(() => true, () => false)).toBe(false)
  const copy = join(dir, 'work.ext4')
  await store.get(owner, copy)
  expect(await readFile(copy, 'utf8')).toBe('one')
  const second = join(dir, 'second.ext4')
  await writeFile(second, 'two')
  await store.put(owner, second)
  await store.get(owner, copy)
  expect(await readFile(copy, 'utf8')).toBe('two')
  // The working copy is its own file.
  await writeFile(copy, 'scribbled')
  await store.get(owner, join(dir, 'again.ext4'))
  expect(await readFile(join(dir, 'again.ext4'), 'utf8')).toBe('two')
  await store.delete(owner)
  expect(await store.has(owner)).toBe(false)
})

test('resize2fs -M report parsing', () => {
  expect(parseResizedBlocks('resize2fs 1.47.0 (5-Feb-2023)\nResizing the filesystem on home.ext4 to 2080 (4k) blocks.\nThe filesystem on home.ext4 is now 2080 (4k) blocks long.\n')).toEqual({ blocks: 2080, blockSize: 4096 })
  expect(parseResizedBlocks('nothing')).toBeNull()
})

const tools = missingImageTools().length === 0 ? test : test.skip

tools('make, shrink and grow a home image round trip', async () => {
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

  const shrunk = await shrinkImage(image)
  expect(shrunk).toBeLessThan(nominal)
  expect((await stat(image)).size).toBe(shrunk)
  await growImage(image, nominal)
  expect((await stat(image)).size).toBe(nominal)
  // Growing the backing file leaves the filesystem to the guest; the image is still consistent.
  const check = await runTool('e2fsck', ['-fn', image])
  expect(check.code).toBe(0)
}, 60_000)
