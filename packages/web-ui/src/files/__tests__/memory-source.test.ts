import { expect, test } from 'bun:test'
import { createMemoryFileSource, dir, file } from '../memory-source'
import { FileBrowserError } from '../types'

function source(offline = false) {
  return createMemoryFileSource({
    platform: 'linux',
    home: '/home/zan',
    offline,
    root: dir({
      home: dir({
        zan: dir({
          Projects: dir({
            demi: dir({
              'README.md': file(120, '2026-09-01T10:00:00Z')
            })
          }),
          locked: dir({}, {
            failure: {
              kind: 'permission',
              message: 'Permission denied'
            }
          }),
          'notes.txt': file(10, '2026-09-02T10:00:00Z'),
        }),
      }),
    }),
  })
}

test('lists a directory with sizes and times', async () => {
  const entries = await source().list('/home/zan')
  expect(entries).toEqual([
    { name: 'Projects', isDirectory: true, modifiedAt: undefined },
    { name: 'locked', isDirectory: true, modifiedAt: undefined },
    {
      name: 'notes.txt',
      isDirectory: false,
      size: 10,
      modifiedAt: '2026-09-02T10:00:00Z'
    },
  ])
})

test('a missing path, a failing directory and an offline device reject with their kinds', async () => {
  await expect(source().list('/home/zan/none')).rejects.toMatchObject(
    { kind: 'not-found' }
  )
  await expect(source().list('/home/zan/notes.txt')).rejects.toMatchObject(
    { kind: 'not-found' }
  )
  await expect(source().list('/home/zan/locked')).rejects.toMatchObject(
    {
      kind: 'permission',
      message: 'Permission denied'
    }
  )
  await expect(source(true).list('/home/zan')).rejects.toBeInstanceOf(FileBrowserError)
  await expect(source(true).list('/home/zan')).rejects.toMatchObject(
    { kind: 'offline' }
  )
})

test('createDirectory makes the directory and any missing above it, and leaves one already there', async () => {
  const s = source()
  await s.createDirectory!('/home/zan/Projects/next/inner')
  expect((await s.list('/home/zan/Projects')).map((entry) => entry.name)).toEqual(
    ['demi', 'next']
  )
  expect((await s.list('/home/zan/Projects/next')).map((entry) => entry.name)).toEqual(['inner'])
  await s.createDirectory!('/home/zan/Projects/demi')
  expect((await s.list('/home/zan/Projects/demi')).map((entry) => entry.name)).toEqual(['README.md'])
  await expect(s.createDirectory!('/home/zan/notes.txt/x')).rejects.toMatchObject(
    { kind: 'other' }
  )
  await expect(s.createDirectory!('/home/zan/locked/x')).rejects.toMatchObject(
    { kind: 'permission' }
  )
})

test('remove deletes a file or a directory with what is in it, and a path with nothing is fine', async () => {
  const s = source()
  await s.remove!('/home/zan/Projects/demi')
  await s.remove!('/home/zan/notes.txt')
  await s.remove!('/home/zan/nothing')
  expect((await s.list('/home/zan')).map((entry) => entry.name)).toEqual(['Projects', 'locked'])
})

test('an upload lands as a file, writes over one only when told, and never over a folder', async () => {
  const s = createMemoryFileSource({
    platform: 'linux',
    home: '/w',
    root: dir({ w: dir({ 'a.txt': file(1, '2026-09-01T10:00:00Z'), docs: dir({}) }) }),
    uploadRate: 1024 * 1024,
  })
  const options = (replace: boolean) => ({ replace, signal: new AbortController().signal, progress: () => {} })
  await s.upload!('/w/b.txt', new File(['bb'], 'b.txt'), options(false))
  expect((await s.list('/w')).find((entry) => entry.name === 'b.txt')?.size).toBe(2)
  await expect(s.upload!('/w/a.txt', new File(['aaa'], 'a.txt'), options(false))).rejects.toMatchObject({ kind: 'exists' })
  await s.upload!('/w/a.txt', new File(['aaa'], 'a.txt'), options(true))
  expect((await s.list('/w')).find((entry) => entry.name === 'a.txt')?.size).toBe(3)
  await expect(s.upload!('/w/docs', new File(['x'], 'docs'), options(true))).rejects.toMatchObject({ kind: 'other' })
})
