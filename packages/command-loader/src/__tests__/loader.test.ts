import { describe, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-local'
import type { DispatchIO } from '@demicodes/shell'
import { bytesStream, collectBytes, decodeUtf8, emptyByteStream, encodeUtf8 } from '@demicodes/utils'
import { buildManifest, createLoader, inMemorySource, inProcessRpc, type Loader } from '../index'
import { memoryStorage, testRoots, transpile } from './fixtures'

/** The loader against a real directory: runtime modules over `host.fs`, rpc in process. */

async function world(withRpc = true) {
  const dir = mkdtempSync(join(tmpdir(), 'command-loader-'))
  const host = new LocalHost(dir, { storeRoot: join(dir, '.store') })
  const roots = testRoots()
  const manifest = await buildManifest(roots, { transpile })
  const loader = await createLoader({
    source: inMemorySource(manifest),
    host,
    rpc: withRpc ? inProcessRpc(roots, { storage: memoryStorage(), host }) : undefined,
  })
  const run = async (root: string, argv: string[], stdin = '', env: Record<string, string> = {}) => {
    const out: Uint8Array[] = []
    const err: Uint8Array[] = []
    const io: DispatchIO = {
      stdin: stdin ? bytesStream(encodeUtf8(stdin)) : emptyByteStream(),
      stdout: (data) => void out.push(typeof data === 'string' ? encodeUtf8(data) : data),
      stderr: (data) => void err.push(typeof data === 'string' ? encodeUtf8(data) : data),
      cwd: dir,
      env,
    }
    const exit = await loader.dispatch(root, argv, io)
    return { exit, stdout: decodeUtf8(await collectBytes(bytesStream(Buffer.concat(out)))), stderr: decodeUtf8(Buffer.concat(err)) }
  }
  return { dir, host, loader, run, dispose: () => rmSync(dir, { recursive: true, force: true }) }
}

describe('dispatch', () => {
  test('a runtime module runs against the host filesystem with the caller cwd and args', async () => {
    const w = await world()
    try {
      await w.host.fs.writeFile('a.txt', encodeUtf8('hello\n'), { cwd: w.dir })
      const result = await w.run('myagent', ['copy', 'a.txt', 'b.txt', '--upper'])
      expect(result).toEqual({ exit: 0, stdout: `copied a.txt -> b.txt in ${w.dir}\n`, stderr: '' })
      expect(decodeUtf8(await w.host.fs.readFile('b.txt', { cwd: w.dir }))).toBe('HELLO\n')
    } finally {
      w.dispose()
    }
  })

  test('stdin streams into a runtime module, env is visible, the exit code is the module\'s', async () => {
    const w = await world()
    try {
      const result = await w.run('myagent', ['echo', '--code', '3'], 'piped bytes', { HOME: '/home/x' })
      expect(result).toEqual({ exit: 3, stdout: 'piped bytes', stderr: 'env HOME=/home/x\n' })
    } finally {
      w.dispose()
    }
  })

  test('an rpc leaf runs its handler through the transport, with stdin and --json', async () => {
    const w = await world()
    try {
      expect(await w.run('myagent', ['note', 'add', 'first'])).toEqual({ exit: 0, stdout: '1 notes\n', stderr: '' })
      expect(await w.run('myagent', ['note', 'add'], 'from stdin')).toEqual({ exit: 0, stdout: '2 notes\n', stderr: '' })
      expect(await w.run('myagent', ['note', 'add', 'third', '--json'])).toEqual({ exit: 0, stdout: '{"count":3}', stderr: '' })
    } finally {
      w.dispose()
    }
  })

  test('a group prints its help; usage errors exit 1 with the reason; an unknown root exits 127', async () => {
    const w = await world()
    try {
      const help = await w.run('myagent', ['note'])
      expect(help.exit).toBe(0)
      expect(help.stdout).toContain('myagent note add')
      const bad = await w.run('myagent', ['copy', 'only-one'])
      expect(bad.exit).toBe(1)
      expect(bad.stderr).toBe('myagent: Invalid value for "to": Invalid input: expected string, received undefined\n')
      const unknown = await w.run('nope', ['x'])
      expect(unknown.exit).toBe(127)
      expect(unknown.stderr).toBe('nope: not a root command of this manifest\n')
    } finally {
      w.dispose()
    }
  })

  test('without a transport, rpc leaves report the missing transport while runtime leaves still run', async () => {
    const w = await world(false)
    try {
      const rpc = await w.run('myagent', ['note', 'add', 'x'])
      expect(rpc.exit).toBe(1)
      expect(rpc.stderr).toContain('no rpc transport')
      await w.host.fs.writeFile('a.txt', encodeUtf8('x'), { cwd: w.dir })
      expect((await w.run('myagent', ['copy', 'a.txt', 'c.txt'])).exit).toBe(0)
    } finally {
      w.dispose()
    }
  })

  test('the loader exposes the manifest and the reconstructed roots', async () => {
    const w = await world()
    try {
      const loader: Loader = w.loader
      expect(Object.keys(loader.manifest.roots)).toEqual(['myagent'])
      expect(loader.roots.map((root) => root.name)).toEqual(['myagent'])
    } finally {
      w.dispose()
    }
  })
})
