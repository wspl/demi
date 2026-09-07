import { expect, test } from 'bun:test'
import { createMemoryFileSource, dir, file } from '../memory-source'
import { FileBrowserError } from '../types'

function source(offline = false) {
  return createMemoryFileSource({
    home: '/home/zan',
    offline,
    root: dir({
      home: dir({
        zan: dir({
          Projects: dir({ demi: dir({ 'README.md': file(120, '2026-09-01T10:00:00Z') }) }),
          locked: dir({}, { failure: { kind: 'permission', message: 'Permission denied' } }),
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
    { name: 'notes.txt', isDirectory: false, size: 10, modifiedAt: '2026-09-02T10:00:00Z' },
  ])
})

test('a missing path, a failing directory and an offline device reject with their kinds', async () => {
  await expect(source().list('/home/zan/none')).rejects.toMatchObject({ kind: 'not-found' })
  await expect(source().list('/home/zan/notes.txt')).rejects.toMatchObject({ kind: 'not-found' })
  await expect(source().list('/home/zan/locked')).rejects.toMatchObject({ kind: 'permission', message: 'Permission denied' })
  await expect(source(true).list('/home/zan')).rejects.toBeInstanceOf(FileBrowserError)
  await expect(source(true).list('/home/zan')).rejects.toMatchObject({ kind: 'offline' })
})

test('createDirectory adds an empty directory once', async () => {
  const s = source()
  await s.createDirectory!('/home/zan/Projects/next')
  expect((await s.list('/home/zan/Projects')).map((entry) => entry.name)).toEqual(['demi', 'next'])
  expect(await s.list('/home/zan/Projects/next')).toEqual([])
  await expect(s.createDirectory!('/home/zan/Projects/next')).rejects.toMatchObject({ kind: 'other' })
  await expect(s.createDirectory!('/home/zan/locked/x')).rejects.toMatchObject({ kind: 'permission' })
})
