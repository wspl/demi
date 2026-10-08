import { expect, test } from 'bun:test'
import { homePath, isValidEntryName, joinPath, normalizePath, parentPath, pathSegments, relativePath } from '../paths'

test('normalizePath collapses slashes and resolves dot segments from the root', () => {
  expect(normalizePath('/Users//zan/./Projects/')).toBe('/Users/alex/Projects')
  expect(normalizePath('/Users/alex/../build')).toBe('/Users/build')
  expect(normalizePath('/../..')).toBe('/')
  expect(normalizePath('relative/dir')).toBe('/relative/dir')
  expect(normalizePath('')).toBe('/')
})

test('joinPath and parentPath agree at the root', () => {
  expect(joinPath('/', 'src')).toBe('/src')
  expect(joinPath('/Users/alex', 'Projects')).toBe('/Users/alex/Projects')
  expect(parentPath('/Users/alex')).toBe('/Users')
  expect(parentPath('/Users')).toBe('/')
  expect(parentPath('/')).toBe('/')
})

test('pathSegments lists the root and every ancestor', () => {
  expect(pathSegments('/Users/alex')).toEqual([
    { name: '/', path: '/' },
    { name: 'Users', path: '/Users' },
    { name: 'zan', path: '/Users/alex' },
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

test('homePath reads a path under the home from ~, and leaves one outside it or without a home whole', () => {
  expect(homePath('/Users/alex/code/shop-app', '/Users/alex')).toBe('~/code/shop-app')
  expect(homePath('/Users/alex', '/Users/alex/')).toBe('~')
  expect(homePath('/Users/alex2/notes', '/Users/alex')).toBe('/Users/alex2/notes')
  expect(homePath('/srv/app', null)).toBe('/srv/app')
})
