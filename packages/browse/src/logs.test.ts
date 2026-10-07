// The page's logs as one daemon leaves them and the next goes on from them,
// in a temporary folder; a few milliseconds.
import { afterEach, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Logs } from './logs'

const folders: string[] = []

afterEach(() => {
  for (const folder of folders.splice(0)) {
    rmSync(folder, { recursive: true, force: true })
  }
})

/** The logs a daemon kept of the browser at `browser`, left in a file, and that file. */
function leftBehind(browser: string): string {
  const folder = mkdtempSync(join(tmpdir(), 'browse-logs-'))
  folders.push(folder)
  const path = join(folder, 'page-logs.json')
  const logs = new Logs()
  logs.add('console', 'log: before the mark')
  logs.mark('reload')
  logs.add('console', 'error: after the mark')
  logs.add('network', 'GET 200 http://127.0.0.1:3323/api/sync 12 ms')
  logs.save(path, browser)
  return path
}

test('a daemon that starts again with the same browser goes on from the logs and the mark the last one left', () => {
  const path = leftBehind('http://127.0.0.1:9222')
  const next = new Logs()
  next.restore(path, 'http://127.0.0.1:9222')
  next.add('console', 'log: after the restart')
  const lines = next.read('console', { all: false, modules: false }).map((line) => line.trim().replace(/^\S+ s\s+/, ''))
  expect([next.since(false), lines]).toEqual(['since the mark reload', ['error: after the mark', 'log: after the restart']])
  expect(next.read('console', { all: true, modules: false })).toHaveLength(3)
})

test('the logs of another browser are not the new browser\'s', () => {
  const path = leftBehind('http://127.0.0.1:9222')
  const next = new Logs()
  next.restore(path, 'http://127.0.0.1:9333')
  expect([next.read('console', { all: true, modules: false }), next.since(false)]).toEqual([[], 'since the tool attached to the browser'])
})
