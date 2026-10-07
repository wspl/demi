// Starts real process groups of `sh` and `sleep`; about 0.3 s.
import { afterEach, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { ownGroupRuns, startGroup, stopGroup, type Group } from './processes'

const folder = mkdtempSync(join(tmpdir(), 'check-processes-'))
const started: Group[] = []

afterEach(async () => {
  for (const group of started.splice(0)) {
    await stopGroup(group, 1_000)
  }
})

function start(command: string[]): Group {
  const { group } = startGroup(command, { cwd: folder, env: process.env, log: join(folder, 'out.log') })
  started.push(group)
  return group
}

/** The process ids of group `pgid`'s members other than its leader, as the leader printed them. */
async function waitForChildren(path: string): Promise<number[]> {
  const deadline = Date.now() + 5_000
  while (Date.now() < deadline) {
    const text = await Bun.file(path).text().catch(() => '')
    const pids = text.trim().split(/\s+/).filter(Boolean).map(Number)
    if (pids.length === 2) {
      return pids
    }
    await Bun.sleep(20)
  }
  throw new Error('the group printed no children')
}

/** Whether process `pid` runs: it exists and has not exited, as a zombie not yet reaped has. */
function runs(pid: number): boolean {
  const ps = Bun.spawnSync(['ps', '-o', 'stat=', '-p', String(pid)], { stdout: 'pipe', stderr: 'ignore' })
  const state = ps.stdout.toString().trim()
  return state !== '' && !state.startsWith('Z')
}

test('stopping a group stops every process in it, children included, and no other process', async () => {
  const pids = join(folder, 'pids')
  // The leader starts two children and waits; a SIGTERM to the leader alone would leave them.
  const group = start(['sh', '-c', `sleep 60 & a=$!; sleep 60 & echo $a $! > ${pids}; wait`])
  const bystander = start(['sleep', '60'])
  const children = await waitForChildren(pids)
  expect(await stopGroup(group, 5_000)).toBe('stopped')
  expect([runs(group.pgid), ...children.map(runs)]).toEqual([false, false, false])
  expect(ownGroupRuns(bystander)).toBe(true)
})

test('a recorded group whose id now names a process started at another time is left alone', async () => {
  const other = start(['sleep', '60'])
  const stale: Group = { ...other, started: 'Thu Jan  1 00:00:00 1970' }
  expect(ownGroupRuns(stale)).toBe(false)
  expect(await stopGroup(stale, 1_000)).toBe('not running')
  expect(runs(other.pgid)).toBe(true)
})

process.on('exit', () => rmSync(folder, { recursive: true, force: true }))
