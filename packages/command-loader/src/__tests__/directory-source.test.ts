import { expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-remote/testing'
import { TEST_COMMAND_CONTEXT } from '@demicodes/shell/testing'
import { emptyByteStream } from '@demicodes/utils'
import { buildManifest } from '../manifest/build'
import { createLoader } from '../loader/loader'
import { directorySource, writeManifestDirectory } from '../loader/source'
import { testRoots, testPackage, testNative } from './fixtures'

test(
  'directorySource: the declaration manifest is read back and invokes an explicit adapter',
  async () => {
    const root = await mkdtemp(join(tmpdir(), 'demi-manifest-dir-'))
    try {
    const host = new LocalHost(root, { storeRoot: join(root, 'store') })
    const manifest = await buildManifest(testRoots(), { packages: [testPackage] })
    const dir = join(root, 'commands', manifest.hash)
    await writeManifestDirectory(manifest, dir, host.fs)

    expect(await host.fs.readdir(dir)).toEqual(['manifest.json'])
    const source = directorySource(dir, host.fs)
    const loader = await createLoader({ source, host, native: testNative(host) })
    expect(loader.manifest).toEqual(manifest)

    await host.fs.writeFile(
      join(root, 'in.txt'),
      new TextEncoder().encode('lower')
    )
    let stdout = ''
    const code = await loader.dispatch('scout', [
      'copy',
      'in.txt',
      'out.txt',
      '--upper'
    ], {
      stdin: emptyByteStream(),
      stdout: (data) => void (stdout += typeof data === 'string'
        ? data
        : new TextDecoder().decode(data)),
      stderr: () => {},
      cwd: root,
      env: {},
      context: TEST_COMMAND_CONTEXT,
    })
    expect(code).toBe(0)
    expect(stdout).toBe(`copied in.txt -> out.txt in ${root}\n`)
    expect(
      new TextDecoder().decode(await host.fs.readFile(join(root, 'out.txt')))
    ).toBe('LOWER')
    } finally { await rm(root, { recursive: true, force: true }) }
  }
)
