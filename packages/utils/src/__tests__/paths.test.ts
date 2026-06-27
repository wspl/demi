import { describe, expect, it } from 'bun:test'
import { dirnamePath, isAbsolutePath, normalizePath } from '../paths'

describe('normalizePath', () => {
  it('collapses . and .. segments', () => {
    expect(normalizePath('/a/./b/../c')).toBe('/a/c')
    expect(normalizePath('a/b/../../c')).toBe('c')
  })

  it('keeps relative paths relative and empties to .', () => {
    expect(normalizePath('a/b')).toBe('a/b')
    expect(normalizePath('')).toBe('.')
    expect(normalizePath('./')).toBe('.')
    expect(normalizePath('../x')).toBe('../x')
  })

  it('preserves absolute roots and clamps above root', () => {
    expect(normalizePath('/')).toBe('/')
    expect(normalizePath('/../..')).toBe('/')
  })

  it('normalizes backslashes and drive letters', () => {
    expect(normalizePath('C:\\a\\b')).toBe('C:/a/b')
    expect(normalizePath('c:/')).toBe('C:/')
  })
})

describe('dirnamePath', () => {
  it('returns the parent directory', () => {
    expect(dirnamePath('/a/b/c')).toBe('/a/b')
    expect(dirnamePath('/a')).toBe('/')
    expect(dirnamePath('a')).toBe('.')
    expect(dirnamePath('/')).toBe('/')
  })

  it('handles drive roots', () => {
    expect(dirnamePath('C:/a')).toBe('C:/')
  })
})

describe('isAbsolutePath', () => {
  it('detects posix and windows absolutes', () => {
    expect(isAbsolutePath('/a')).toBe(true)
    expect(isAbsolutePath('C:/a')).toBe(true)
    expect(isAbsolutePath('C:\\a')).toBe(true)
    expect(isAbsolutePath('a/b')).toBe(false)
    expect(isAbsolutePath('./a')).toBe(false)
  })
})
