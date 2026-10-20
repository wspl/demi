/**
 * A repository's history as the Change view's Commits mode shows it: the local branches, a branch's
 * commits by first parents a page at a time, and one commit's files against
 * its first parent. A host serves each as a kept entry it follows, so a
 * commit, a checkout or a new branch made anywhere shows at once.
 */
import { failureShown, type Showing } from './file-cache'
import type { ChangeFile, ChangeSides } from './changes'
import { FileBrowserError, type FileContents } from './types'

/** The local branches, most recently committed first, and the one checked out, or the commit a detached HEAD names. */
export interface BranchList {
  branches: readonly string[]
  /** Null while HEAD is detached. */
  current: string | null
  /** The commit a detached HEAD names; null while a branch is checked out. */
  head: string | null
}

/** One commit as a history lists it: its id, its whole message, its author's name and its time. */
export interface LoggedCommit {
  id: string
  message: string
  author: string
  /** An ISO 8601 time. */
  time: string
}

/** One page of a history, newest first; `more` while older commits follow. */
export interface CommitLogPage {
  commits: readonly LoggedCommit[]
  more: boolean
}

/** A file a commit changed, with the one letter git's log gives it: A, M, D or R. */
export interface CommitFile extends ChangeFile {
  status: string
}

/** One commit: its entry with its parents, its files against its first parent, and the local branches it is on, the checked-out one first. */
export interface CommitDetails {
  commit: LoggedCommit & { parents: readonly string[] }
  files: readonly CommitFile[]
  /** The list stopped at the host's limit. */
  truncated: boolean
  branches: readonly string[]
}

/**
 * The history of the repository a working tree is in, each part a kept
 * entry a view shows until it lets go: what a read answers that nothing is
 * there, a branch deleted or a commit the repository lacks, is the answer
 * `gone`.
 */
export interface CommitHistorySource {
  showBranches(): Showing<BranchList>
  /** One page of `branch`'s history, HEAD's for null, after the commit `after` names, from the newest for ''. */
  showLog(branch: string | null, after: string): Showing<CommitLogPage>
  /** The commit an id or an abbreviation of at least 7 hex digits names. */
  showCommit(commit: string): Showing<CommitDetails>
  /** The two sides of one file of a commit, by its path relative to the working tree's root; a renamed file's old side at `from`. */
  showSides(commit: string, path: string, from?: string): Showing<ChangeSides>
  /** Each file as `commit` has it, by its path relative to the root, for previews and Download; absent when the host serves no bytes. */
  contents?(commit: string): FileContents
}

/**
 * The full hash of the commit `commit`, an id or an abbreviation, as the
 * history reads it now, as a link's Copy Commit Hash asks; the commit is
 * held only for the read. Rejects with a `FileBrowserError` when the
 * repository has no such commit or it could not be read.
 */
export async function readCommitHash(history: CommitHistorySource, commit: string): Promise<string> {
  const showing = history.showCommit(commit)
  try {
    await showing.retry()
    const failure = failureShown(showing.entry)
    if (failure)
      throw new FileBrowserError(failure.kind, failure.message, { requestId: failure.requestId ?? null })
    const id = showing.entry.value?.commit.id
    if (id === undefined)
      throw new FileBrowserError('not-found')
    return id
  } finally {
    showing.release()
  }
}

/** A commit's id as people read it: its first seven hex digits. */
export function shortCommit(id: string): string {
  return id.slice(0, 7)
}

/** A message's summary: its first line. */
export function commitSummary(message: string): string {
  return message.split('\n', 1)[0]!.trim()
}

/** What the branch menu names for the list shown: the branch, the checked-out one for null, or `HEAD (3f2a9c1)` while HEAD is detached. */
export function shownBranchName(shown: string | null, branches: BranchList | undefined): string {
  if (shown !== null)
    return shown
  if (branches?.current)
    return branches.current
  return branches?.head ? `HEAD (${shortCommit(branches.head)})` : 'HEAD'
}

/**
 * Whether the commit list starts with Uncommitted Changes: only while the
 * branch it shows is the one checked out, or HEAD, as the working tree's
 * changes are against HEAD alone.
 */
export function listsUncommitted(shown: string | null, branches: BranchList | undefined): boolean {
  return shown === null || shown === branches?.current
}

/**
 * The branch whose list a linked commit opens in, by the branches it is on, the checked-out one first and
 * then by their last commit: the branch shown when it holds the commit,
 * else the checked-out one, null, else the most recently committed one
 * that holds it; `undefined` when no local branch holds it, which shows the
 * commit alone.
 */
export function linkedCommitBranch(
  shown: string | null,
  current: string | null,
  containing: readonly string[],
): string | null | undefined {
  if (containing.length === 0)
    return undefined
  if (shown !== null && containing.includes(shown))
    return shown === current ? null : shown
  if (current !== null && containing.includes(current))
    return null
  return containing.find((branch) => branch !== current)
}
