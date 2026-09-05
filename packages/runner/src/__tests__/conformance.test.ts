import { expect, test } from 'bun:test'
import { mkdtemp, realpath } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { bundleForTinyjs, tinyjsBinary } from '../testing'

// The machine layer runs only on tinyjs, so its conformance run does too: the
// suite from @demicodes/shell/testing is bundled with the Host and run on
// the bare binary.
test('the machine layer passes the Host conformance suite on tinyjs', async () => {
  const work = await realpath(await mkdtemp(join(tmpdir(), 'demi-machine-')))
  const bundle = join(work, 'conformance.mjs')
  await bundleForTinyjs(join(import.meta.dir, 'conformance', 'main.ts'), bundle)
  const root = join(work, 'root')
  await Bun.$`mkdir -p ${root}`
  const run = Bun.spawnSync([tinyjsBinary(), bundle], {
    env: { PATH: process.env.PATH ?? '/usr/bin:/bin', HOME: work, HOST_CONFORMANCE_ROOT: root },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const report = `${run.stdout.toString()}${run.stderr.toString()}`
  expect(report, report).not.toContain('FAIL')
  expect(run.exitCode, report).toBe(0)
}, 120_000)
