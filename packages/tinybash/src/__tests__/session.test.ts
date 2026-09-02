import { describe, expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-local'
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
})
