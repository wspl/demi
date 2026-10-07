// What `up`, `down` and `runner` share about the slot's processes: the
// environment they start with, the log each writes, and how the tool waits
// until one is ready or has failed.
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import net from 'node:net'
import { dirname, join, resolve } from 'node:path'
import { parseEnv } from 'node:util'
import { CheckFailure, type Context } from './command'
import { ownGroupRuns, type Started } from './processes'
import { slotPaths, type Slot } from './slot'
import { readState, type ServerName } from './state'

/**
 * The environment of a process the tool starts: the daemon's, without its
 * `DEMI_*` settings, which may be stale, and with the caller's and
 * `extra`. A runner refuses a `DEMI_*` variable it does not know, and the
 * servers read the slot's `.env` themselves.
 */
export function childEnv(context: Context, extra: Record<string, string>): Record<string, string | undefined> {
  const env: Record<string, string | undefined> = {}
  for (const [name, value] of Object.entries(process.env)) {
    if (!name.startsWith('DEMI_')) {
      env[name] = value
    }
  }
  return { ...env, ...context.env, ...extra }
}

/** The log of the process `name`, emptied for a new start. */
export function freshLog(slot: Slot, name: string): string {
  const path = join(slotPaths(slot).logs, `${name}.log`)
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, '')
  return path
}

/** Whether a program listens on `port` of this machine. */
export function listening(port: number): Promise<boolean> {
  return new Promise((done) => {
    const probe = net.connect(port, '127.0.0.1')
    probe.once('connect', () => {
      probe.destroy()
      done(true)
    })
    probe.once('error', () => done(false))
  })
}

/** The last lines of a log, for a failure to show. */
export function tail(path: string, count = 30): string {
  const lines = readFileSync(path, 'utf8').trimEnd().split('\n')
  return lines.slice(-count).map((line) => `  ${line}`).join('\n')
}

/** How often a starting process is looked at. */
const POLL_MS = 200

/**
 * Waits until `ready` answers something, failing when the process ends
 * first or `timeoutMs` passes; the failure shows the end of its log.
 */
export async function untilReady<T>(
  what: string,
  started: Started,
  ready: () => Promise<T | null>,
  timeoutMs: number,
): Promise<T> {
  let exit: number | null = null
  void started.exited.then((code) => {
    exit = code
  })
  const deadline = Date.now() + timeoutMs
  for (;;) {
    const answer = await ready()
    if (answer !== null) {
      return answer
    }
    if (exit !== null) {
      throw new CheckFailure(`${what} exited with ${exit} before it was ready. Its log, ${started.group.log}, ends:\n${tail(started.group.log)}`)
    }
    if (Date.now() >= deadline) {
      throw new CheckFailure(`${what} was not ready within ${timeoutMs / 1000} s. Its log, ${started.group.log}, ends:\n${tail(started.group.log)}`)
    }
    await Bun.sleep(POLL_MS)
  }
}

/** The first match of `pattern` in the log at `path`, or null. */
export function lineOf(path: string, pattern: RegExp): RegExpExecArray | null {
  return pattern.exec(readFileSync(path, 'utf8'))
}

/** Whether `url` answers with a success. */
export async function answersOk(url: string): Promise<boolean> {
  try {
    const answer = await fetch(url, { signal: AbortSignal.timeout(2_000) })
    return answer.ok
  } catch {
    // Not listening yet, or not answering yet: not ready.
    return false
  }
}

/** The slot's server `name` when the tool started it and it still runs. */
export function runningServer(slot: Slot, name: ServerName) {
  const group = readState(slot).servers[name]
  return group && ownGroupRuns(group) ? group : undefined
}

/**
 * The user's own checkout, which holds the `.env` a slot borrows: a slot is
 * a git worktree, whose `.git` file names its git directory, whose
 * `commondir` names the checkout's. Null when `root` is the checkout itself.
 */
export function userCheckout(root: string): string | null {
  const dotGit = join(root, '.git')
  if (statSync(dotGit).isDirectory()) {
    return null
  }
  const text = readFileSync(dotGit, 'utf8')
  const gitdir = /^gitdir: (.+)$/m.exec(text)?.[1]
  if (!gitdir) {
    return null
  }
  const common = resolve(gitdir, readFileSync(join(gitdir, 'commondir'), 'utf8').trim())
  return dirname(common)
}

/** The value `name` has in the `.env` at `root`, if any. */
export function dotEnvValue(root: string, name: string): string | undefined {
  const path = join(root, '.env')
  return existsSync(path) ? parseEnv(readFileSync(path, 'utf8'))[name] : undefined
}
