import { expect, test } from 'bun:test'
import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/shell/node'
import { emptyByteStream } from '@demicodes/utils'
import { buildManifest } from '../manifest/build'
import { createLoader } from '../loader/loader'
import { directorySource, writeManifestDirectory } from '../loader/source'
import { testRoots, transpile } from './fixtures'

// A manifest kept as files, modules imported by path: the route tinyjs takes on
// a target, proven here under Bun with the same loader.
test('directorySource: the manifest is read back from files and runtime modules import by path', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-manifest-dir-'))
  const host = new LocalHost(root, { storeRoot: join(root, 'store') })
  const manifest = await buildManifest(testRoots(), { transpile })
  const dir = join(root, 'commands', manifest.hash)
  await writeManifestDirectory(manifest, dir, host.fs)

  expect(await host.fs.readdir(join(dir, 'modules'))).toEqual(Object.keys(manifest.modules).map((hash) => `${hash}.mjs`).sort())
  const source = directorySource(dir, host.fs)
  const loader = await createLoader({ source, host })
  expect(loader.manifest).toEqual(manifest)

  await host.fs.writeFile(join(root, 'in.txt'), new TextEncoder().encode('lower'))
  let stdout = ''
  const code = await loader.dispatch('scout', ['copy', 'in.txt', 'out.txt', '--upper'], {
    stdin: emptyByteStream(),
    stdout: (data) => void (stdout += typeof data === 'string' ? data : new TextDecoder().decode(data)),
    stderr: () => {},
    cwd: root,
    env: {},
  })
  expect(code).toBe(0)
  expect(stdout).toBe(`copied in.txt -> out.txt in ${root}\n`)
  expect(new TextDecoder().decode(await host.fs.readFile(join(root, 'out.txt')))).toBe('LOWER')
})
