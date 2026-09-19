import { expect, test } from 'bun:test'
import { baseName, isValidEntryName, joinPath, normalizePath, parentPath, pathSegments, relativePath } from '../paths'

test('normalizePath collapses slashes and resolves dot segments from the root', () => {
  expect(normalizePath('/Users//zan/./Projects/')).toBe('/Users/zan/Projects')
  expect(normalizePath('/Users/zan/../build')).toBe('/Users/build')
  expect(normalizePath('/../..')).toBe('/')
  expect(normalizePath('relative/dir')).toBe('/relative/dir')
  expect(normalizePath('')).toBe('/')
})

test('joinPath, parentPath and baseName agree at the root', () => {
  expect(joinPath('/', 'src')).toBe('/src')
  expect(joinPath('/Users/zan', 'Projects')).toBe('/Users/zan/Projects')
  expect(parentPath('/Users/zan')).toBe('/Users')
  expect(parentPath('/Users')).toBe('/')
  expect(parentPath('/')).toBe('/')
  expect(baseName('/Users/zan')).toBe('zan')
  expect(baseName('/')).toBe('')
})

test('pathSegments lists the root and every ancestor', () => {
  expect(pathSegments('/Users/zan')).toEqual([
    { name: '/', path: '/' },
    { name: 'Users', path: '/Users' },
    { name: 'zan', path: '/Users/zan' },
  ])
  expect(pathSegments('/')).toEqual([{ name: '/', path: '/' }])
})

test('entry names may not be empty, dots or contain a separator', () => {
  expect(isValidEntryName('src')).toBe(true)
  expect(isValidEntryName('  ')).toBe(false)
  expect(isValidEntryName('.')).toBe(false)
  expect(isValidEntryName('..')).toBe(false)
  expect(isValidEntryName('a/b')).toBe(false)
})


test('relativePath reads a path from a root, and leaves one outside it whole', () => {
  expect(relativePath('/work/demi', '/work/demi/src/a.ts')).toBe('src/a.ts')
  expect(relativePath('/work/demi/', '/work/demi')).toBe('')
  expect(relativePath('/work/demi', '/work/demi-other/a.ts')).toBe('/work/demi-other/a.ts')
  expect(relativePath('/', '/etc/hosts')).toBe('etc/hosts')
})
