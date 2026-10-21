import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { createApp, effectScope, shallowReactive, shallowRef } from 'vue'
import { useWorkingSides, type SidesOf } from '../change-sides'
import type { ChangeSetSource, ChangeSides } from '../changes'
import { useCommitLog } from '../commit-log'
import type { Showing, ShownEntry } from '../file-cache'
import { HostFiles } from '../file-cache'
import type { CommitHistorySource, CommitLogPage } from '../history'
import { keptHistory } from '../kept-source'
import { SimulatedWatch } from '../memory-source'
import { FileBrowserError } from '../types'
import { WatchChannel } from '../watch-channel'

// The Change view's Commits mode as the page keeps it (`packages/web-ui/src/files/history.ts`): a page of the commit list that could not be read waits
// for Retry, a file that becomes a staged rename reads its old side at its
// earlier path, a file's sides at a commit's full id are read once, and
// history read outside a repository is read again once one is made there.
// Cost: no I/O, fake timers; a few milliseconds.

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

async function settle(): Promise<void> {
  for (let tick = 0; tick < 20; tick++)
    await Promise.resolve()
}

/** The page's app, whose context a view's composables run in. */
const app = createApp({})

function inView<T>(run: () => T): T {
  return app.runWithContext(() => effectScope().run(run)!)
}

/** An entry as a test answers it by hand: what a view reads of it, writable. */
type Answered<T> = { -readonly [Field in keyof ShownEntry<T>]: ShownEntry<T>[Field] }

/** An entry a test answers by hand. */
function shown<T>(retry: () => void = () => {}): { showing: Showing<T>; entry: Answered<T> } {
  const entry = shallowReactive<Answered<T>>({ value: undefined, answer: null, failure: null, reading: false, outdated: false, unwatched: null })
  const showing: Showing<T> = { entry, retry: async () => retry(), release: () => {}, away: () => {}, back: () => {} }
  return { showing, entry }
}

function page(from: number, more: boolean): CommitLogPage {
  const commits = Array.from({ length: 2 }, (_, at) => ({ id: `c${from + at}`, message: `Commit ${from + at}`, author: 'Alex', time: '2026-10-01T00:00:00.000Z' }))
  return { commits, more }
}

test('a later page that could not be read says so, and the list’s next ask reads it again', async () => {
  const pages = new Map<string, Answered<CommitLogPage>>()
  const retried: string[] = []
  const unused = (): never => {
    throw new Error('not shown')
  }
  const history: CommitHistorySource = {
    showBranches: unused,
    showCommit: unused,
    showSides: unused,
    showLog: (_branch, after) => {
      const { showing, entry } = shown<CommitLogPage>(() => retried.push(after))
      pages.set(after, entry)
      return showing
    },
  }
  const log = inView(() => useCommitLog(() => history, () => null))
  pages.get('')!.value = page(0, true)
  pages.get('')!.answer = 'value'
  await settle()
  log.loadMore()
  await settle()
  pages.get('c1')!.failure = { kind: 'other', message: 'The device is offline.' }
  await settle()
  expect(log.commits.value.map((commit) => commit.id)).toEqual(['c0', 'c1'])
  expect(log.failure.value).toBe('The device is offline.')
  expect(log.loading.value).toBe(false)

  log.loadMore()
  expect(retried).toEqual(['c1'])
  pages.get('c1')!.failure = null
  pages.get('c1')!.value = page(2, false)
  pages.get('c1')!.answer = 'value'
  await settle()
  expect(log.failure.value).toBe(null)
  expect(log.commits.value.map((commit) => commit.id)).toEqual(['c0', 'c1', 'c2', 'c3'])
})

test('a file that becomes a staged rename reads its old side again, at its earlier path', async () => {
  const asked: SidesOf[] = []
  const released: SidesOf[] = []
  const set: ChangeSetSource = {
    files: [],
    showSides: (path, from) => {
      const sides = from === undefined ? { path } : { path, from }
      asked.push(sides)
      const { showing } = shown<ChangeSides>()
      return { ...showing, release: () => released.push(sides) }
    },
  }
  const change = shallowRef<SidesOf>({ path: 'new.txt' })
  inView(() => useWorkingSides(() => set, () => change.value))
  change.value = { path: 'new.txt', from: 'old.txt' }
  await settle()
  expect(asked).toEqual([{ path: 'new.txt' }, { path: 'new.txt', from: 'old.txt' }])
  expect(released).toEqual([{ path: 'new.txt' }])
})

test('a file’s sides at a commit’s full id are read once; an abbreviation’s again when the head moves', async () => {
  const files = new HostFiles()
  const watched = new SimulatedWatch()
  const watch = new WatchChannel(files, watched.open)
  const reads: string[] = []
  const history = keptHistory({
    branches: async () => ({ value: { branches: [], current: 'main', head: null }, readAt: watched.readAt() }),
    log: async () => ({ value: { commits: [], more: false }, readAt: watched.readAt() }),
    commit: async () => { throw new Error('not read') },
    async sides(commit, path) {
      reads.push(commit)
      return { value: { original: `${path} before`, modified: `${path} after` }, readAt: watched.readAt() }
    },
  }, '/w', { files, watch })
  const full = '3f2a9c1d'.padEnd(40, '0')
  const atFull = history.showSides(full, 'a.ts')
  const atAbbreviation = history.showSides('3f2a9c1', 'a.ts')
  await settle()
  expect(atFull.entry.value).toEqual({ original: 'a.ts before', modified: 'a.ts after' })
  expect(reads).toEqual([full, '3f2a9c1'])

  watched.moveHead('/w')
  await settle()
  jest.advanceTimersByTime(5_000)
  await settle()
  expect(reads).toEqual([full, '3f2a9c1', '3f2a9c1'])
  // Shown again, it is still current.
  atFull.release()
  history.showSides(full, 'a.ts')
  await settle()
  expect(reads).toEqual([full, '3f2a9c1', '3f2a9c1'])
  atAbbreviation.release()
})

test('history read outside a repository is read again when one is made there', async () => {
  const files = new HostFiles()
  const watched = new SimulatedWatch()
  const watch = new WatchChannel(files, watched.open)
  let repository = false
  const history = keptHistory({
    async branches() {
      if (!repository)
        throw new FileBrowserError('other', 'Not a Git repository.')
      return { value: { branches: ['main'], current: 'main', head: null }, readAt: watched.readAt() }
    },
    log: async () => ({ value: { commits: [], more: false }, readAt: watched.readAt() }),
    commit: async () => { throw new Error('not read') },
    sides: async () => { throw new Error('not read') },
  }, '/w', { files, watch })
  const branches = history.showBranches()
  await settle()
  expect(branches.entry.failure?.message).toBe('Not a Git repository.')

  // `git init` and a first commit: the runner's head subscription, rebound to the new repository, says so.
  repository = true
  watched.moveHead('/w')
  await settle()
  expect(branches.entry.value).toEqual({ branches: ['main'], current: 'main', head: null })
})
