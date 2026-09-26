// Runs `bun test` with the arguments given, in a temporary directory of its
// own that goes once the tests end: whatever a test leaves there, such as a
// runner's state with its copies of native packages, a run leaves nothing.
//
// The tests also run as a process group of their own, which ends with them.
// A program a test started and never stopped, such as the runner of a test
// that failed before its cleanup, would otherwise outlive the run and still
// write into the directory while it is removed.
import { existsSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

/** How long what the tests left running gets to stop after SIGTERM. */
const GRACE_MS = 5_000

const directory = mkdtempSync(join(tmpdir(), 'demi-tests-'))
const tests = Bun.spawn([process.execPath, 'test', ...Bun.argv.slice(2)], {
  env: { ...process.env, TMPDIR: directory, TEMP: directory, TMP: directory },
  stdio: ['inherit', 'inherit', 'inherit'],
  // The tests lead a new process group, which every program they start joins
  // unless it makes one of its own.
  detached: true,
})
// The tests left the terminal's process group, so an interrupt reaches them
// through the run; the run still ends their group and removes the directory.
process.on('SIGINT', () => signalTests('SIGINT'))
process.on('SIGTERM', () => signalTests('SIGTERM'))
const code = await tests.exited
await endLeftovers()
rmSync(directory, { recursive: true, force: true })
if (existsSync(directory)) {
  console.error(`scripts/test.ts: could not remove ${directory}`)
  process.exit(code === 0 ? 1 : code)
}
process.exit(code)

/**
 * Sends `signal` to every process in the tests' group, and answers whether
 * any was there: none is left once the group is empty.
 */
function signalTests(signal: NodeJS.Signals | 0): boolean {
  try {
    process.kill(-tests.pid, signal)
    return true
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ESRCH')
      return false
    throw error
  }
}

/**
 * Ends what the tests left running: SIGTERM, which lets a runner end its own
 * jobs, then SIGKILL for whatever is still there after the grace period. A
 * process that has exited but that no parent has reaped still counts, so the
 * grace period can pass in full.
 */
async function endLeftovers(): Promise<void> {
  if (!signalTests('SIGTERM'))
    return
  const deadline = Date.now() + GRACE_MS
  while (Date.now() < deadline && signalTests(0))
    await Bun.sleep(50)
  signalTests('SIGKILL')
}
