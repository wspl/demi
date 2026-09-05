import { describe, expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-virtual/testing'
import type { HostFileSystem } from '@demicodes/shell'
import { isAbsolutePath, normalizePath } from '@demicodes/utils'
import { runTinybash, type ShellState } from '../index'
import { stubRoots } from '../testing'

/**
 * The parse-first namespace decision is sound: a script the check accepts
 * touches nothing outside the namespace when it runs. Random scripts built
 * from the constructs that move or compute paths (`cd` that may fail, globs,
 * `..`, `$PWD`, variables, chains, the mutating builtins) are run against a
 * filesystem that records every path it is asked for.
 */

const SEEDS = Number(process.env.TINYBASH_FUZZ_ROUNDS ?? 3000)

function rng(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const pick = <T>(random: () => number, items: readonly T[]): T => items[Math.floor(random() * items.length)]!

const PATHS = [
  'a', 'a/b', 'b', 'c', 'missing', 'missing/x', '.', '..', '../outside.txt', '../o', '../o/x.txt', '../..',
  '*', '*/..', '*/../..', '*/*/..', 'a/*', 'a/*/..', '../*', '*/../../outside.txt', 'm*', '[ab]', 'a/../..',
  '~/a', '~/../outside.txt', '~/..', '$X', '$X/..', '$X/../..', '$PWD/..', '$PWD/a', './a', 'a/b/../../..',
  'f.txt', 'a/g.txt', '../a', 'a/../c', 'c/../../o',
]

function path(random: () => number): string {
  const p = pick(random, PATHS)
  return random() < 0.15 ? `${p}/${pick(random, PATHS)}` : p
}

function command(random: () => number): string {
  const p = path(random)
  const q = path(random)
  switch (Math.floor(random() * 22)) {
    case 0: case 1: case 2: return `cd ${p}`
    case 3: return `mkdir ${p}`
    case 4: return `mkdir -p ${p}`
    case 5: return `rm -rf ${p}`
    case 6: return `mv ${p} ${q}`
    case 7: return `cp -r ${p} ${q}`
    case 8: return `touch ${p}`
    case 9: return `cat ${p}`
    case 10: return `ls ${p}`
    case 11: return `echo x > ${p}`
    case 12: return `cat < ${p}`
    case 13: return `X=${p}`
    case 14: return `echo $PWD`
    case 15: return `test -d ${p}`
    case 16: return `find ${p} -name x`
    case 17: return `demi file read ${p}`
    case 18: return `cat ${p} | head -n 1`
    case 19: return `cat ${p} | cd ${q}`
    case 20: return `X=${p} cd ${q}`
    default: return `echo x >> ${p}`
  }
}

function script(random: () => number): string {
  const statements: string[] = []
  const count = 1 + Math.floor(random() * 4)
  for (let i = 0; i < count; i++) {
    let statement = command(random)
    while (random() < 0.35) statement += ` ${pick(random, ['&&', '||'])} ${command(random)}`
    statements.push(statement)
  }
  return statements.join(pick(random, ['; ', '\n']))
}

/** Wraps a filesystem so every path it is asked for is recorded as an absolute path. */
function recording(fs: HostFileSystem, touched: string[]): HostFileSystem {
  const pathArgs: Record<string, number[]> = { cp: [0, 1], mv: [0, 1], link: [0, 1], symlink: [1] }
  return new Proxy(fs, {
    get(target, property, receiver) {
      const value = Reflect.get(target, property, receiver)
      if (typeof value !== 'function') return value
      return (...args: unknown[]) => {
        const options = args[args.length - 1]
        const cwd = typeof options === 'object' && options !== null && 'cwd' in options ? String((options as { cwd?: string }).cwd ?? '/') : '/'
        for (const index of pathArgs[String(property)] ?? [0]) {
          const arg = args[index]
          if (typeof arg === 'string') touched.push(normalizePath(isAbsolutePath(arg) ? arg : `${cwd}/${arg}`))
        }
        return (value as (...a: unknown[]) => unknown).apply(target, args)
      }
    },
  })
}

function world() {
  const dir = mkdtempSync(join(tmpdir(), 'tinybash-fuzz-'))
  const home = join(dir, 'home')
  for (const d of ['home', 'home/a', 'home/a/b', 'home/c', 'o']) mkdirSync(join(dir, d))
  writeFileSync(join(home, 'f.txt'), 'f\n')
  writeFileSync(join(home, 'a/g.txt'), 'g\n')
  writeFileSync(join(dir, 'outside.txt'), 'LEAKED\n')
  writeFileSync(join(dir, 'o/x.txt'), 'LEAKED\n')
  const host = new LocalHost(home, { storeRoot: join(dir, 'store') })
  return { dir, home, host, dispose: () => rmSync(dir, { recursive: true, force: true }) }
}

describe('namespace check soundness', () => {
  test('an accepted script touches only the namespace', async () => {
    const failures: string[] = []
    const crashes: string[] = []
    let accepted = 0
    for (let seed = 1; seed <= SEEDS; seed++) {
      const random = rng(seed)
      const text = script(random)
      const w = world()
      try {
        const touched: string[] = []
        const state: ShellState = { cwd: w.home, home: w.home, vars: { HOME: w.home } }
        const { roots, dispatch } = stubRoots({ demi: { paths: (argv) => (argv[0] === 'file' && argv[1] === 'read' && argv[2] !== undefined ? [argv[2]] : []) } })
        let result: Awaited<ReturnType<typeof runTinybash>>
        try {
          result = await runTinybash({
            script: text,
            roots,
            namespace: [w.home],
            dispatch,
            fs: recording(w.host.fs, touched),
            state,
            io: { stdout: () => {}, stderr: () => {} },
            identity: { user: 'demi', group: 'demi' },
          })
        } catch (error) {
          crashes.push(`seed ${seed}: ${String(error)}\n  ${text}`)
          continue
        }
        // The check itself may look a `cd` target up; that lookup must stay inside too.
        const leaked = touched.filter((p) => p !== '/dev/null' && p !== w.home && !p.startsWith(`${w.home}/`))
        if (leaked.length > 0) failures.push(`seed ${seed} (${result.kind}): touched ${[...new Set(leaked)].join(', ')}\n  ${text.replace(/\n/g, '\n  ')}`)
        if (result.kind === 'outside') continue
        accepted++
        if (state.cwd !== w.home && !state.cwd.startsWith(`${w.home}/`)) failures.push(`seed ${seed}: cwd left at ${state.cwd}\n  ${text}`)
      } finally {
        w.dispose()
      }
    }
    expect(accepted).toBeGreaterThan(SEEDS / 20)
    expect(failures.slice(0, 20).join('\n')).toBe('')
    expect(crashes.slice(0, 10).join('\n')).toBe('')
  }, 600_000)
})
