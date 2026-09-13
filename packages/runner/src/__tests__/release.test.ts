import { expect, test } from 'bun:test'
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { NATIVE_TARGETS } from '@demicodes/command-protocol'
import { runnerReleaseSchema } from '@demicodes/runner-protocol/release'

test('runner releases verify immutable artifacts before advancing the manifest', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-release-'))
  const artifacts = join(root, 'artifacts')
  const output = join(root, 'releases')
  const filename = (target: string) => target.includes('windows') ? 'demi-runner.exe' : 'demi-runner'
  async function publish() {
    const child = Bun.spawn([
      process.execPath, '--conditions', 'development',
      resolve(import.meta.dir, '../../runtime/release.ts'),
      '--artifacts', artifacts, '--output', output,
    ], { stdout: 'pipe', stderr: 'pipe' })
    const [code, stdout, stderr] = await Promise.all([
      child.exited, new Response(child.stdout).text(), new Response(child.stderr).text(),
    ])
    return { code, stdout, stderr }
  }
  try {
    for (const target of NATIVE_TARGETS) {
      const directory = join(artifacts, target, 'release')
      await mkdir(directory, { recursive: true })
      await writeFile(join(directory, filename(target)), `fixture ${target}`)
    }
    const first = await publish()
    expect(first.code, first.stderr).toBe(0)
    const original = await readFile(join(output, 'manifest.json'), 'utf8')
    const manifest = runnerReleaseSchema.parse(JSON.parse(original))
    const repeated = await publish()
    expect(repeated.code, repeated.stderr).toBe(0)
    const target = NATIVE_TARGETS[0]
    await writeFile(join(output, manifest.release, target, filename(target)), 'corrupt')
    const rejected = await publish()
    expect(rejected.code).not.toBe(0)
    expect(rejected.stderr).toContain('corrupt')
    expect(await readFile(join(output, 'manifest.json'), 'utf8')).toBe(original)
    expect((await readdir(output)).sort()).toEqual([manifest.release, 'manifest.json'].sort())
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
