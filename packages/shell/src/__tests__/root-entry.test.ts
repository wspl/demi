import { expect, test } from 'bun:test'
import { HostBackedFileSystem, type Host } from '../index'

test('root entry exposes browser-safe Host contract and HostBackedFileSystem class', async () => {
  const host: Pick<Host, 'root'> = { root: '/' }
  expect(host.root).toBe('/')

  const fs = new HostBackedFileSystem({ root: '/tmp', spawn: async () => { throw new Error('not used') } } as Host)
  expect(typeof fs.resolvePath).toBe('function')
  expect(fs.resolvePath('/a', 'b')).toBe('/a/b')
})
