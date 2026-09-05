import { describe, expect, test } from 'bun:test'
import { buildManifest, rootPaths, treeFromManifest } from '../index'
import { testRoots, transpile } from './fixtures'

/** `rootPaths`: the path marks answer tinybash's question, from a declared or a reconstructed tree. */

describe('rootPaths', () => {
  test('names the path arguments of an invocation and nothing else', () => {
    const paths = rootPaths(testRoots()).get('scout')!
    expect(paths(['copy', 'src/a.txt', '../b.txt', '--upper'])).toEqual(['src/a.txt', '../b.txt'])
    expect(paths(['echo', '--code', '2'])).toEqual([])
    expect(paths(['note', 'add', '/not/a/path'])).toEqual([])
  })

  test('an argv the tree cannot parse, or help, names no paths', () => {
    const paths = rootPaths(testRoots()).get('scout')!
    expect(paths(['copy', 'only-one'])).toEqual([])
    expect(paths(['copy', '--help'])).toEqual([])
    expect(paths(['nope'])).toEqual([])
    expect(paths([])).toEqual([])
  })

  test('the marks survive the manifest round trip', async () => {
    const manifest = await buildManifest(testRoots(), { transpile })
    const paths = rootPaths(treeFromManifest(manifest, undefined)).get('scout')!
    expect(paths(['copy', 'x', 'y'])).toEqual(['x', 'y'])
  })
})
