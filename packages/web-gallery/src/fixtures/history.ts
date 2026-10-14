import { shallowRef } from 'vue'
import type { KeptAnswer, Showing, ShownEntry } from '@demicodes/web-ui/files/file-cache'
import type { BranchList, CommitDetails, CommitFile, CommitHistorySource, CommitLogPage, LoggedCommit } from '@demicodes/web-ui/files/history'

/** A page of a log, as the runner pages it. */
const PAGE = 50
const HOUR_MS = 60 * 60 * 1000

/** A full commit id, made up but stable: forty hex digits from `seed`. */
function commitId(seed: number): string {
  let id = ''
  let value = (seed * 2654435761 + 97) >>> 0
  while (id.length < 40) {
    value = (value * 1103515245 + 12345) >>> 0
    id += value.toString(16).padStart(8, '0')
  }
  return id.slice(0, 40)
}

const SUMMARIES = [
  'Fix the cart total when a coupon empties it',
  'Add the cart page',
  'Show the shipping estimate under the total',
  'Keep the session cookie across the checkout redirect',
  'Rename the cookie to demi_session',
  'Document the checkout flow in the guide',
  'Retry a failed payment once before saying so',
  'Move the price formatting into one helper',
]
const AUTHORS = ['Alex Kim', 'Sam Ortiz', 'Priya Natarajan']
/** The files the commits change, each a text the diff shows. */
const PATHS = ['src/cart/total.ts', 'src/cart/page.tsx', 'src/auth/cookie.ts', 'docs/guide.md', 'src/cart/total.test.ts']

interface FixtureCommit extends LoggedCommit {
  parents: string[]
  files: CommitFile[]
}

/** The `index`th commit of a history, `hours` before the gallery opened, after `parent`. */
function fixtureCommit(seed: number, index: number, hours: number, parent: string | null, now: number): FixtureCommit {
  const summary = SUMMARIES[index % SUMMARIES.length]!
  const body = index % 3 === 0 ? `${summary}\n\nThe total read the coupon before the items, so an order\nwhose coupon covered it all showed a negative total.` : summary
  const count = 1 + (index % 3)
  const files: CommitFile[] = Array.from({ length: count }, (_, at) => {
    const path = PATHS[(index + at) % PATHS.length]!
    const kind = parent === null || (index + at) % 7 === 3 ? 'added' : 'modified'
    return { path, status: kind === 'added' ? 'A' : 'M', kind, added: 2 + at, removed: kind === 'added' ? 0 : 1 }
  })
  return {
    id: commitId(seed),
    message: body,
    author: AUTHORS[index % AUTHORS.length]!,
    time: new Date(now - hours * HOUR_MS).toISOString(),
    parents: parent === null ? [] : [parent],
    files,
  }
}

/** A text of `lines` lines naming `path`, as one side of a commit's diff. */
function sideText(path: string, lines: number, changed: boolean): string {
  return Array.from({ length: lines }, (_, line) =>
    `// ${path}, line ${line + 1}${changed && line % 4 === 1 ? ' (changed in this commit)' : ''}`).join('\n') + '\n'
}

/** A kept entry the gallery's history answers at once from its fixture, which follows the fixture as it changes. */
function fixed<T>(read: () => { value: T | undefined; answer: KeptAnswer }): Showing<T> {
  // Its getters read the fixture's state, which is reactive: a view follows it.
  const entry: ShownEntry<T> = {
    get value() {
      return read().value
    },
    get answer() {
      return read().answer
    },
    failure: null,
    reading: false,
    unwatched: null,
  }
  return { entry, retry: () => Promise.resolve(), release: () => {}, away: () => {}, back: () => {} }
}

/**
 * The gallery workspace's repository: `main`, checked out, with 64 commits,
 * so the list loads a second page as it scrolls; `feat/cart`, three commits
 * on top of main's tenth; `docs/guide`, older; and a commit no branch holds,
 * as a rebase leaves one. `commit` adds a commit to the branch checked out,
 * as one made in a terminal would, and the list follows.
 */
export function galleryHistory() {
  const now = Date.now()
  const main: FixtureCommit[] = []
  let parent: string | null = null
  for (let index = 63; index >= 0; index -= 1) {
    const made = fixtureCommit(index + 1, index, 2 + index * 5, parent, now)
    main.unshift(made)
    parent = made.id
  }
  const base = main[9]!
  const cart: FixtureCommit[] = []
  parent = base.id
  for (let index = 2; index >= 0; index -= 1) {
    const made = fixtureCommit(1000 + index, index + 1, 1 + index * 3, parent, now)
    made.message = ['Add a coupon field to the cart', 'Validate the coupon code as it is typed', 'Apply the coupon before tax'][index]!
    cart.unshift(made)
    parent = made.id
  }
  const guide = main.slice(30)
  const orphan = fixtureCommit(5000, 4, 30, main[1]!.id, now)
  orphan.message = 'Try a cookie-free checkout\n\nAbandoned: the payment provider needs the session.'
  const histories = shallowRef<Record<string, FixtureCommit[]>>({
    main,
    'feat/cart': [...cart, ...main.slice(9)],
    'docs/guide': guide,
  })
  const all = () => new Map([...Object.values(histories.value).flat(), orphan].map((commit) => [commit.id, commit]))
  const branches = (): BranchList => ({
    branches: Object.keys(histories.value).sort((a, b) => Date.parse(histories.value[b]![0]!.time) - Date.parse(histories.value[a]![0]!.time)),
    current: 'main',
    head: null,
  })
  const find = (commit: string) => [...all().values()].filter((candidate) => candidate.id.startsWith(commit.toLowerCase()))

  const source: CommitHistorySource = {
    showBranches: () => fixed(() => ({ value: branches(), answer: 'value' })),
    showLog: (branch, after) => fixed<CommitLogPage>(() => {
      const list = histories.value[branch ?? 'main']
      if (!list)
        return { value: undefined, answer: 'gone' }
      const start = after === '' ? 0 : list.findIndex((commit) => commit.id === after) + 1
      const commits = list.slice(start, start + PAGE)
      return { value: { commits, more: start + PAGE < list.length }, answer: 'value' }
    }),
    showCommit: (commit) => fixed<CommitDetails>(() => {
      const [found, other] = find(commit)
      if (!found || other)
        return { value: undefined, answer: 'gone' }
      const holding = Object.entries(histories.value).filter(([, list]) => list.some((candidate) => candidate.id === found.id)).map(([name]) => name)
      const ordered = branches().branches.filter((name) => holding.includes(name))
      return {
        value: {
          commit: { id: found.id, message: found.message, author: found.author, time: found.time, parents: found.parents },
          files: found.files,
          truncated: false,
          branches: [...ordered.filter((name) => name === 'main'), ...ordered.filter((name) => name !== 'main')],
        },
        answer: 'value',
      }
    }),
    showSides: (commit, path) => fixed(() => {
      const file = all().get(commit)?.files.find((candidate) => candidate.path === path)
      if (!file)
        return { value: undefined, answer: 'gone' }
      return {
        value: {
          original: file.kind === 'added' ? '' : sideText(path, 12, false),
          modified: file.kind === 'deleted' ? '' : sideText(path, 12 + file.added - file.removed, true),
        },
        answer: 'value',
      }
    }),
  }
  /** A commit on the branch checked out, made now, as the agent's shell or the user's terminal would. */
  function commit(message: string): void {
    const list = histories.value.main!
    const made = fixtureCommit(9000 + list.length, list.length, 0, list[0]!.id, Date.now())
    made.message = message
    histories.value = { ...histories.value, main: [made, ...list] }
  }
  return { source, commit, ids: { latest: () => histories.value.main![0]!.id, cart: cart[0]!.id, orphan: orphan.id, root: main.at(-1)!.id } }
}

export type GalleryHistory = ReturnType<typeof galleryHistory>

/** The gallery's latest commit on `main`, by its abbreviation, as a message links it. */
export const LINKED_COMMIT = commitId(1).slice(0, 7)

/** The gallery's one repository, which every Commits specimen and the panel's Change tab read. */
export const sharedHistory = galleryHistory()
