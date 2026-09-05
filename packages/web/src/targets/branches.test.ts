import { expect, test } from 'bun:test'
import { branchNameError } from './branches'

test('accepts common branch names and rejects invalid reference syntax', () => {
  for (const name of ['main', 'feature/header', 'codex/ui-2', '修复布局']) {
    expect(branchNameError(name)).toBeNull()
  }
  for (const name of [
    '',
    'has space',
    '../main',
    'a..b',
    'main.lock',
    '.hidden',
    'a/.hidden',
    'a//b',
    'a/',
    '/main',
    'main.',
    '@',
    '@{1}',
    'a:b',
    'a\\b',
    '-option',
  ]) {
    expect(branchNameError(name)).not.toBeNull()
  }
})
