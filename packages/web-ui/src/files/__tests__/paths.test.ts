import { expect, test } from 'bun:test'
import { homePath, isValidEntryName, joinPath, normalizePath, parentPath, pathSegments, relativePath, typedPath } from '../paths'

test('normalizePath collapses slashes and resolves dot segments from the root', () => {
  expect(normalizePath('/Users//zan/./Projects/')).toBe('/Users/alex/Projects')
  expect(normalizePath('/Users/alex/../build')).toBe('/Users/build')
  expect(normalizePath('/../..')).toBe('/')
  expect(normalizePath('relative/dir')).toBe('/relative/dir')
  expect(normalizePath('')).toBe('/')
})

test('joinPath and parentPath agree at the root', () => {
  expect(joinPath('/', 'src')).toBe('/src')
  expect(joinPath('/Users/zan', 'Projects')).toBe('/Users/alex/Projects')
  expect(parentPath('/Users/zan')).toBe('/Users')
  expect(parentPath('/Users')).toBe('/')
  expect(parentPath('/')).toBe('/')
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

test('homePath reads a path under the home from ~, and leaves one outside it or without a home whole', () => {
  expect(homePath('/Users/alex/code/ledable-app', '/Users/zan')).toBe('~/code/ledable-app')
  expect(homePath('/Users/zan', '/Users/alex/')).toBe('~')
  expect(homePath('/Users/zan2/notes', '/Users/zan')).toBe('/Users/zan2/notes')
  expect(homePath('/srv/app', null)).toBe('/srv/app')
})

test('a typed path names the home with ~, starts a relative one from the workspace, and keeps an absolute one', () => {
  const places = { home: '/Users/zan', root: '/Users/alex/Projects/demi' }
  expect(typedPath('~', places)).toBe('/Users/zan')
  expect(typedPath('~/notes/todo.md', places)).toBe('/Users/alex/notes/todo.md')
  expect(typedPath('docs/guide.md', places)).toBe('/Users/alex/Projects/demi/docs/guide.md')
  expect(typedPath('/etc/hosts', places)).toBe('/etc/hosts')
  // Only a leading `~` alone or before a slash is the home, as in a shell.
  expect(typedPath('~notes', places)).toBe('/Users/alex/Projects/demi/~notes')
  expect(typedPath('~', { home: '/Users/zan' })).toBe('/Users/zan')
})
