import { describe, expect, test } from 'bun:test'
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { runTinybash } from '../index'
import { stubRoots } from '../testing'
import { CASES, type CorpusCase } from './corpus/cases'
import { buildFixture } from './corpus/fixture'
import { concatBytes, decodeLatin1, toBytes } from '@demicodes/utils'
import { type Golden, HOME_TOKEN, goldenPath, normalizeHome, runOnBash } from './corpus/generate'

/**
 * The equivalence corpus: tinybash against the goldens real GNU bash produced
 * (`corpus/generate.ts`). On Linux, `TINYBASH_CHECK_GOLDENS=1` additionally
 * re-derives every golden from bash and fails on drift.
 */

const collect = () => {
  const chunks: Uint8Array[] = []
  const write = (data: string | Uint8Array) => {
    chunks.push(toBytes(data))
  }
  return { write, text: () => decodeLatin1(concatBytes(chunks)) }
}

export async function runCase(testCase: CorpusCase, identity: { user: string; group: string }): Promise<Golden> {
  const dir = mkdtempSync(join(tmpdir(), 'tinybash-corpus-'))
  const home = join(dir, 'home')
  try {
    const host = new LocalHost(home, { storeRoot: join(dir, 'store') })
    await host.fs.mkdir(home)
    await buildFixture(host.fs, home)
    const { roots, dispatch } = stubRoots({
      demi: { paths: (argv) => (argv[0] === 'file' && argv[1] === 'read' ? argv.slice(2) : []) },
      scout: {},
    })
    const stdout = collect()
    const stderr = collect()
    const result = await runTinybash({
      script: testCase.script,
      roots,
      namespace: [home],
      dispatch,
      fs: host.fs,
      state: { cwd: home, home, vars: { HOME: home, PATH: `${dir}/bin:/usr/bin:/bin` } },
      io: { stdout: stdout.write, stderr: stderr.write },
      identity,
    })
    if (result.kind === 'outside') throw new Error(`corpus case ${testCase.name} is outside the subset: ${result.message}`)
    return {
      exit: result.exitCode,
      stdout: normalizeHome(stdout.text(), home),
      stderr: normalizeHome(stderr.text(), home),
      user: identity.user,
      group: identity.group,
    }
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
}

function readGolden(name: string): Golden | null {
  const path = goldenPath(name)
  if (!existsSync(path)) return null
  return JSON.parse(readFileSync(path, 'utf8')) as Golden
}

describe('equivalence corpus', () => {
  for (const testCase of CASES) {
    const golden = readGolden(testCase.name)
    const skip = golden === null || (testCase.linuxOnly === true && process.platform !== 'linux')
    test.skipIf(skip)(testCase.name, async () => {
      const actual = await runCase(testCase, { user: golden!.user, group: golden!.group })
      expect(actual.stdout).toBe(golden!.stdout)
      expect(actual.stderr).toBe(golden!.stderr)
      expect(actual.exit).toBe(golden!.exit)
    })
  }

  test('every case has a golden', () => {
    const missing = CASES.filter((c) => readGolden(c.name) === null).map((c) => c.name)
    expect(missing).toEqual([])
  })

  test('case names are unique, case-insensitively', () => {
    expect(new Set(CASES.map((c) => c.name.toLowerCase())).size).toBe(CASES.length)
  })

  test.skipIf(process.platform !== 'linux' || !process.env.TINYBASH_CHECK_GOLDENS)('goldens match real bash', async () => {
    for (const testCase of CASES) {
      const fresh = await runOnBash(testCase)
      expect({ name: testCase.name, ...fresh }).toEqual({ name: testCase.name, ...readGolden(testCase.name)! })
    }
  }, 600_000)
})

export { HOME_TOKEN }
