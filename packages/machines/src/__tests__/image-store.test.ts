import { mkdir, mkdtemp, open, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { atomicJson, copyMachineImage, DirMachineImageStore } from '../machine-image-store'

test.skipIf(process.platform !== 'linux')('copying sparse Cloud storage retains holes and boundary data', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'demi-sparse-'))
  try {
    const source = join(directory, 'source')
    const destination = join(directory, 'copy')
    const bytes = 64 * 1024 ** 2
    const file = await open(source, 'wx')
    try {
      await file.write(Buffer.from('head'), 0, 4, 0)
      await file.write(Buffer.from('tail'), 0, 4, bytes - 4)
    } finally {
      await file.close()
    }
    await copyMachineImage(source, destination)
    const metadata = await stat(destination)
    expect(metadata.size).toBe(bytes)
    expect(metadata.blocks * 512).toBeLessThan(1024 ** 2)
    const copy = await open(destination, 'r')
    try {
      for (const [position, expected] of [[0, 'head'], [bytes - 4, 'tail']] as const) {
        const buffer = Buffer.alloc(4)
        await copy.read(buffer, 0, 4, position)
        expect(buffer.toString()).toBe(expected)
      }
    } finally {
      await copy.close()
    }
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})

test('partial disk publication preserves the complete pair and retains only two committed generations', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'demi-store-'))
  try {
    const source = join(directory, 'source')
    const images = join(directory, 'images')
    const store = new DirMachineImageStore(images)
    await mkdir(source)
    await writeFile(join(source, 'system.ext4'), 'system')
    await writeFile(join(source, 'home.ext4'), 'home')
    const state = { generation: 'first', baseVersion: 'base', resetId: null, systemBytes: 1024, homeBytes: 1024 }
    await store.publish('device', state, source)
    await rm(join(source, 'home.ext4'))
    await expect(store.publish('device', { ...state, generation: 'partial' }, source)).rejects.toThrow()
    expect(await store.read('device')).toEqual(state)
    const recovered = join(directory, 'recovered')
    await store.copy('device', state, recovered)
    expect(await readFile(join(recovered, 'home.ext4'), 'utf8')).toBe('home')
    await store.publish('device', { ...state, generation: 'second' }, recovered)
    await store.publish('device', { ...state, generation: 'third' }, recovered)
    expect((await readdir(join(images, 'device/generations'))).sort()).toEqual(['second', 'third'])
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})

test('failed JSON serialization preserves the published file and removes staging', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'demi-json-'))
  try {
    const path = join(directory, 'current.json')
    await atomicJson(path, { generation: 'complete' })
    await expect(atomicJson(path, { value: 1n })).rejects.toThrow()
    expect(JSON.parse(await readFile(path, 'utf8'))).toEqual({ generation: 'complete' })
    expect(await readdir(directory)).toEqual(['current.json'])
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
