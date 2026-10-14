import { expect, test } from 'bun:test'
import { directoryParts, homePath, isValidEntryName, joinPath, normalizePath, parentPath, pathSegments, relativePath, typedPath } from '../paths'

test('normalizePath collapses slashes and resolves dot segments from the root', () => {
  expect(normalizePath('/Users//alex/./Projects/')).toBe('/Users/alex/Projects')
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
    { name: 'alex', path: '/Users/alex' },
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
  expect(homePath('/Users/alex/code/sample-app', '/Users/alex')).toBe('~/code/sample-app')
  expect(homePath('/Users/alex', '/Users/alex/')).toBe('~')
  expect(homePath('/Users/alex2/notes', '/Users/alex')).toBe('/Users/alex2/notes')
  expect(homePath('/srv/app', null)).toBe('/srv/app')
})

test('a typed path names the home with ~, starts a relative one from the workspace, and keeps an absolute one', () => {
  const places = { home: '/Users/alex', root: '/Users/alex/Projects/demi' }
  expect(typedPath('~', places)).toBe('/Users/alex')
  expect(typedPath('~/notes/todo.md', places)).toBe('/Users/alex/notes/todo.md')
  expect(typedPath('docs/guide.md', places)).toBe('/Users/alex/Projects/demi/docs/guide.md')
  expect(typedPath('/etc/hosts', places)).toBe('/etc/hosts')
  // Only a leading `~` alone or before a slash is the home, as in a shell.
  expect(typedPath('~notes', places)).toBe('/Users/alex/Projects/demi/~notes')
  expect(typedPath('~', { home: '/Users/alex' })).toBe('/Users/alex')
})

test('a directory row reads by its name, its parent beside it, the home as ~', () => {
  const home = '/Users/alex'
  expect(directoryParts('/Users/alex/Projects/demi', home)).toEqual({ name: 'demi', location: '~/Projects' })
  expect(directoryParts('/Users/alex/demi', home)).toEqual({ name: 'demi', location: '~' })
  expect(directoryParts('/srv/app/', home)).toEqual({ name: 'app', location: '/srv' })
  expect(directoryParts('/tmp', home)).toEqual({ name: 'tmp', location: '/' })
  expect(directoryParts('/Users/alex', home)).toEqual({ name: '~', location: '' })
  expect(directoryParts('/', home)).toEqual({ name: '/', location: '' })
  expect(directoryParts('/home/build/app', null)).toEqual({ name: 'app', location: '/home/build' })
})
