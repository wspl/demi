import { expect, test } from 'bun:test'
import dayjs from 'dayjs'
import { createFileBrowserHistory, filterEntries, nextSort, sortEntries } from '../file-browser-state'
import { entryKind, formatBytes, formatModified } from '../format'
import type { FileBrowserEntry } from '../types'

const entries: FileBrowserEntry[] = [
  { name: 'file10.txt', isDirectory: false, size: 300, modifiedAt: '2026-09-01T00:00:00Z' },
  { name: '.git', isDirectory: true, modifiedAt: '2026-09-03T00:00:00Z' },
  { name: 'file2.txt', isDirectory: false, size: 100, modifiedAt: '2026-09-02T00:00:00Z' },
  { name: 'src', isDirectory: true, modifiedAt: '2026-08-01T00:00:00Z' },
]
const names = (list: FileBrowserEntry[]) => list.map((entry) => entry.name)

test('directories lead and names sort naturally', () => {
  expect(names(sortEntries(entries, { key: 'name', direction: 'asc' }))).toEqual(['.git', 'src', 'file2.txt', 'file10.txt'])
  expect(names(sortEntries(entries, { key: 'name', direction: 'desc' }))).toEqual(['src', '.git', 'file10.txt', 'file2.txt'])
})

test('time and size sort within each kind and fall back to the name', () => {
  expect(names(sortEntries(entries, { key: 'modifiedAt', direction: 'desc' }))).toEqual(['.git', 'src', 'file2.txt', 'file10.txt'])
  expect(names(sortEntries(entries, { key: 'size', direction: 'asc' }))).toEqual(['.git', 'src', 'file2.txt', 'file10.txt'])
  expect(names(sortEntries(entries, { key: 'size', direction: 'desc' }))).toEqual(['.git', 'src', 'file10.txt', 'file2.txt'])
})

test('hidden entries hide until asked for; the query matches anywhere in the name', () => {
  expect(names(filterEntries(entries, '', false))).toEqual(['file10.txt', 'file2.txt', 'src'])
  expect(names(filterEntries(entries, '', true))).toHaveLength(4)
  expect(names(filterEntries(entries, 'ILE1', true))).toEqual(['file10.txt'])
})

test('nextSort cycles a column through ascending, descending and the default order', () => {
  expect(nextSort({ key: null, direction: 'asc' }, 'name')).toEqual({ key: 'name', direction: 'asc' })
  expect(nextSort({ key: 'name', direction: 'asc' }, 'name')).toEqual({ key: 'name', direction: 'desc' })
  expect(nextSort({ key: 'name', direction: 'desc' }, 'name')).toEqual({ key: null, direction: 'asc' })
  expect(nextSort({ key: 'name', direction: 'desc' }, 'size')).toEqual({ key: 'size', direction: 'asc' })
  expect(names(sortEntries(entries, { key: null, direction: 'desc' }))).toEqual(['.git', 'src', 'file2.txt', 'file10.txt'])
})

test('history pushes, walks back and forward, and drops the forward stack on a new push', () => {
  const history = createFileBrowserHistory('/a')
  history.push('/a/b')
  history.push('/a/b/c')
  expect(history.canBack).toBe(true)
  expect(history.canForward).toBe(false)
  expect(history.back()).toBe('/a/b')
  expect(history.back()).toBe('/a')
  expect(history.back()).toBeNull()
  expect(history.forward()).toBe('/a/b')
  history.push('/x')
  expect(history.canForward).toBe(false)
  expect(history.current).toBe('/x')
  history.push('/x')
  expect(history.back()).toBe('/a/b')
  history.reset('/r')
  expect(history.current).toBe('/r')
  expect(history.canBack).toBe(false)
})

test('bytes and times format for a column', () => {
  expect(formatBytes(12)).toBe('12 B')
  expect(formatBytes(1536)).toBe('1.5 KB')
  expect(formatBytes(20 * 1024 * 1024)).toBe('20 MB')
  const now = dayjs('2026-09-07T15:00:00')
  expect(formatModified('2026-09-07T09:05:00', now)).toBe('09:05')
  expect(formatModified('2026-03-02T09:05:00', now)).toBe('Mar 2, 09:05')
  expect(formatModified('2025-03-02T09:05:00', now)).toBe('2025-03-02')
  expect(formatModified(undefined, now)).toBe('')
})

test('the Type column reads the extension', () => {
  expect(entryKind('src', true)).toBe('Folder')
  expect(entryKind('index.ts', false)).toBe('TS file')
  expect(entryKind('.zshrc', false)).toBe('File')
  expect(entryKind('Makefile', false)).toBe('File')
  expect(entryKind('archive.', false)).toBe('File')
})
