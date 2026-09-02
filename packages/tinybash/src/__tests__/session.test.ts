import { describe, expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { runTinybash, type ShellState } from '../index'
import { stubRoots } from '../testing'

/** Session state across tool calls, parse-first with nothing run, and cancellation. */

function world() {
  const dir = mkdtempSync(join(tmpdir(), 'tinybash-session-'))
  const home = join(dir, 'home')
  mkdirSync(home)
  const host = new LocalHost(home, { storeRoot: join(dir, 'store') })
  const state: ShellState = { cwd: home, home, vars: { HOME: home } }
  const { roots, dispatch, calls } = stubRoots({ demi: {} })
  let out = ''
  const run = (script: string, signal?: AbortSignal) =>
    runTinybash({
      script,
      roots,
      namespace: [home],
      dispatch,
      fs: host.fs,
      state,
      io: { stdout: (d) => void (out += typeof d === 'string' ? d : new TextDecoder().decode(d)), stderr: () => {} },
      identity: { user: 'demi', group: 'demi' },
      signal,
    })
  return { home, host, state, calls, run, output: () => out, reset: () => void (out = ''), dispose: () => rmSync(dir, { recursive: true, force: true }) }
}

describe('session state', () => {
  test('cd and assignments persist across tool calls; prefix assignments do not', async () => {
    const w = world()
    try {
      await w.host.fs.mkdir(`${w.home}/src`)
      await w.run('cd src; X=1')
      expect(w.state.cwd).toBe(`${w.home}/src`)
      expect(w.state.vars.X).toBe('1')
      await w.run('Y=2 demi run; pwd; echo "$X$Y"')
      expect(w.state.vars.Y).toBeUndefined()
      expect(w.output()).toBe(`demi run\n${w.home}/src\n1\n`)
      expect(w.calls[0]?.cwd).toBe(`${w.home}/src`)
    } finally {
      w.dispose()
    }
  })
})

describe('parse first', () => {
  test('a script whose last statement is outside runs nothing', async () => {
    const w = world()
    try {
      const result = await w.run('echo started > marker.txt; X=changed; cd src; cat /etc/passwd')
      expect(result.kind).toBe('outside')
      expect(await w.host.fs.exists(`${w.home}/marker.txt`)).toBe(false)
      expect(w.state.vars.X).toBeUndefined()
      expect(w.state.cwd).toBe(w.home)
      expect(w.output()).toBe('')
    } finally {
      w.dispose()
    }
  })
})

describe('cancellation', () => {
  test('an aborted signal ends the running statement and skips the rest', async () => {
    const w = world()
    try {
      const controller = new AbortController()
      const { roots, dispatch } = stubRoots({ demi: {} })
      let started = 0
      const slowDispatch: typeof dispatch = async (root, argv, io) => {
        started++
        controller.abort()
        return dispatch(root, argv, io)
      }
      const result = await runTinybash({
        script: 'demi first; echo never; demi second',
        roots,
        namespace: [w.home],
        dispatch: slowDispatch,
        fs: w.host.fs,
        state: w.state,
        io: { stdout: (d) => void (w.output(), d), stderr: () => {} },
        identity: { user: 'demi', group: 'demi' },
        signal: controller.signal,
      })
      expect(started).toBe(1)
      expect(result).toEqual({ kind: 'ran', exitCode: 130 })
    } finally {
      w.dispose()
    }
  })

  test('a builtin reading an endless pipe stops at the abort with status 130', async () => {
    const w = world()
    try {
      const controller = new AbortController()
      let chunks = 0
      const { roots } = stubRoots({ demi: {} })
      // A root command that writes for as long as its reader takes chunks.
      const endless: Parameters<typeof runTinybash>[0]['dispatch'] = async (_root, _argv, io) => {
        for (;;) {
          chunks++
          await io.stdout(new TextEncoder().encode('line\n'))
        }
      }
      let out = ''
      const result = await runTinybash({
        script: 'demi stream | cat; echo never',
        roots,
        namespace: [w.home],
        dispatch: endless,
        fs: w.host.fs,
        state: w.state,
        io: {
          stdout: (d) => {
            out += typeof d === 'string' ? d : new TextDecoder().decode(d)
            if (out.length >= 20) controller.abort()
          },
          stderr: () => {},
        },
        identity: { user: 'demi', group: 'demi' },
        signal: controller.signal,
      })
      expect(result).toEqual({ kind: 'ran', exitCode: 130 })
      expect(out).not.toContain('never')
      expect(chunks).toBeLessThan(10)
    } finally {
      w.dispose()
    }
  })

  test('a tree walk stops at the abort', async () => {
    const w = world()
    try {
      for (let i = 0; i < 20; i++) mkdirSync(join(w.home, `d${i}`))
      const controller = new AbortController()
      let readdirs = 0
      const fs = new Proxy(w.host.fs, {
        get(target, key) {
          const value = Reflect.get(target, key) as unknown
          if (key !== 'readdir' || typeof value !== 'function') return value
          return (...args: unknown[]) => {
            if (++readdirs === 3) controller.abort()
            return (value as (...a: unknown[]) => unknown).apply(target, args)
          }
        },
      })
      const { roots, dispatch } = stubRoots({ demi: {} })
      let out = ''
      const result = await runTinybash({
        script: 'find .; echo never',
        roots,
        namespace: [w.home],
        dispatch,
        fs,
        state: w.state,
        io: { stdout: (d) => void (out += typeof d === 'string' ? d : new TextDecoder().decode(d)), stderr: () => {} },
        identity: { user: 'demi', group: 'demi' },
        signal: controller.signal,
      })
      expect(result).toEqual({ kind: 'ran', exitCode: 130 })
      expect(readdirs).toBe(3)
      expect(out).not.toContain('never')
    } finally {
      w.dispose()
    }
  })
})
