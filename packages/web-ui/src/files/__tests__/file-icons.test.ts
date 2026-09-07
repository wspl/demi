import { expect, test } from 'bun:test'
import manifest from 'material-icon-theme/dist/material-icons.json'
import { fileIconName, landmarkIcon, type FileIconTheme } from '../file-icons'

const theme = manifest as FileIconTheme

test('folders resolve by name and fall back to the plain folder', () => {
  expect(fileIconName(theme, 'src', true)).toBe('folder-src')
  expect(fileIconName(theme, '.git', true)).toBe('folder-git')
  expect(fileIconName(theme, 'node_modules', true)).toBe('folder-node')
  expect(fileIconName(theme, 'Projects', true)).toBe('folder-project')
  expect(fileIconName(theme, 'demi', true)).toBe('folder')
})

test('files match the whole name before the longest suffix, case-insensitively', () => {
  expect(fileIconName(theme, 'package.json', false)).toBe('nodejs')
  expect(fileIconName(theme, 'README.md', false)).toBe('readme')
  expect(fileIconName(theme, 'index.ts', false)).toBe('typescript')
  expect(fileIconName(theme, 'app.test.ts', false)).toBe('test-ts')
  expect(fileIconName(theme, 'types.d.ts', false)).toBe('typescript-def')
  expect(fileIconName(theme, 'App.vue', false)).toBe('vue')
  expect(fileIconName(theme, 'LICENSE', false)).toBe('license')
  expect(fileIconName(theme, 'notes', false)).toBe('file')
})

test('the root and the home are known by their place, not their name', () => {
  const source = { platform: 'macos', home: '/Users/zan' } as const
  expect(landmarkIcon('/', source)).toBe('folder-macos')
  expect(landmarkIcon('/Users/zan', source)).toBe('folder-home')
  expect(landmarkIcon('/Users', source)).toBeUndefined()
})
