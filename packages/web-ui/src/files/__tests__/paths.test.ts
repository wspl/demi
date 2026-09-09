import { expect, test } from 'bun:test'
import {
  baseName,
  displayPath,
  isValidEntryName,
  joinPath,
  normalizePath,
  parentPath,
  pathSegments
} from '../paths'

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

test('displayPath shortens the home directory to a tilde', () => {
  expect(displayPath('/Users/zan', '/Users/zan')).toBe('~')
  expect(displayPath('/Users/zan/Projects', '/Users/zan')).toBe('~/Projects')
  expect(displayPath('/Users/zander', '/Users/zan')).toBe('/Users/zander')
  expect(displayPath('/srv', undefined)).toBe('/srv')
})
