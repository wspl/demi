// The slot's own processes (product-checks.md § The slot's servers): each
// server or runner the tool starts leads a process group of its own, which
// the tool records and later stops as a whole, and never a process found by
// its name, so the user's servers and other slots' are never touched.
import { openSync, closeSync } from 'node:fs'

/** A process group the tool started, as the slot's state records it. */
export interface Group {
  /** The group's id, which is its leader's process id. */
  pgid: number
  /**
   * When the leader started, as the system reports it: a recorded group
   * whose id now names a process that started at another time is not the
   * tool's, since the system reused the id.
   */
  started: string
  command: string[]
  /** The file that holds the group's output. */
  log: string
}

export interface Started {
  group: Group
  /** Resolves with the leader's exit code. */
  exited: Promise<number>
}

/** Starts `command` as the leader of a new process group, its output appended to `log`. */
export function startGroup(
  command: string[],
  options: { cwd: string, env: Record<string, string | undefined>, log: string },
): Started {
  const output = openSync(options.log, 'a')
  try {
    const child = Bun.spawn(command, {
      cwd: options.cwd,
      env: options.env,
      stdio: ['ignore', output, output],
      // A new session, whose group the child leads and its children join:
      // the group outlives the tool's daemon, and stopping it reaches them.
      detached: true,
    })
    child.unref()
    const group: Group = { pgid: child.pid, started: startTime(child.pid) ?? '', command, log: options.log }
    return { group, exited: child.exited }
  } finally {
    // The child holds its own copy of the file.
    closeSync(output)
  }
}

/**
 * When process `pid` started, as `ps` prints it, or null when no such
 * process runs. Neither Bun nor Node offers a process's start time, and on
 * macOS no file holds it, so this asks `ps`.
 */
export function startTime(pid: number): string | null {
  const ps = Bun.spawnSync(['ps', '-o', 'lstart=', '-p', String(pid)], { stdout: 'pipe', stderr: 'ignore' })
  const text = ps.stdout.toString().trim()
  return ps.exitCode === 0 && text ? text : null
}

/**
 * Sends `signal` to every process of group `pgid`; answers whether any was
 * there to run. macOS refuses a signal with EPERM to a group whose
 * processes have all exited but are not yet reaped, which run no more.
 */
function signalGroup(pgid: number, signal: NodeJS.Signals | 0): boolean {
  try {
    process.kill(-pgid, signal)
    return true
  } catch (error) {
    if (error instanceof Error && 'code' in error && (error.code === 'ESRCH' || error.code === 'EPERM')) {
      return false
    }
    throw error
  }
}

/**
 * Whether `group` still runs and is the one the tool started. A group id
 * stays reserved while any process of the group runs, so a group whose
 * leader has exited is still the tool's; a running leader must have started
 * when the record says.
 */
export function ownGroupRuns(group: Group): boolean {
  if (!signalGroup(group.pgid, 0)) {
    return false
  }
  const leader = startTime(group.pgid)
  return leader === null || leader === group.started
}

export type StopOutcome = 'not running' | 'stopped' | 'killed'

/**
 * Stops `group`: SIGTERM, which lets a server stop what it started, then
 * SIGKILL for whatever still runs after `graceMs`. A group that no longer
 * runs, or whose id now names another process, is left alone.
 */
export async function stopGroup(group: Group, graceMs: number): Promise<StopOutcome> {
  if (!ownGroupRuns(group)) {
    return 'not running'
  }
  signalGroup(group.pgid, 'SIGTERM')
  if (await groupEnds(group.pgid, graceMs)) {
    return 'stopped'
  }
  signalGroup(group.pgid, 'SIGKILL')
  // SIGKILL cannot be refused; the wait only covers the system reaping.
  await groupEnds(group.pgid, 5_000)
  return 'killed'
}

/** Waits up to `ms` for group `pgid` to have no process; answers whether it ended. */
async function groupEnds(pgid: number, ms: number): Promise<boolean> {
  const deadline = Date.now() + ms
  while (signalGroup(pgid, 0)) {
    if (Date.now() >= deadline) {
      return false
    }
    await Bun.sleep(50)
  }
  return true
}
