/**
 * How VS Code's Git marks a file, from git's two status letters for it (the
 * `XY` of `git status --porcelain`: the index against HEAD, then the working
 * tree against the index).
 *
 * VS Code files a path under one or two of its resource groups, each with a
 * status. Its explorer then lets a later group's mark replace an earlier one's,
 * in the order index, untracked, working tree, conflicts. So:
 *
 * - A staged new file edited again (`AM`) shows the working tree's M.
 * - A staged type change alone (`T `) shows nothing, since VS Code files no
 *   index resource for it.
 *
 * The letters are VS Code's own; the words are its words in Demi's sentence
 * style.
 */
import { t } from './locales/text'

/** VS Code's `Status` for a resource, the ignored one aside: git lists no ignored file here. */
export type GitResourceStatus =
  | 'index-modified'
  | 'index-added'
  | 'index-deleted'
  | 'index-renamed'
  | 'index-copied'
  | 'modified'
  | 'deleted'
  | 'untracked'
  | 'intent-to-add'
  | 'intent-to-rename'
  | 'type-changed'
  | 'both-deleted'
  | 'added-by-us'
  | 'deleted-by-them'
  | 'added-by-them'
  | 'deleted-by-us'
  | 'both-added'
  | 'both-modified'

/** A conflict's pair, each a resource of VS Code's merge group. */
const CONFLICTS: Record<string, GitResourceStatus> = {
  DD: 'both-deleted',
  AU: 'added-by-us',
  UD: 'deleted-by-them',
  UA: 'added-by-them',
  DU: 'deleted-by-us',
  AA: 'both-added',
  UU: 'both-modified',
}

/** The working tree's letter, a resource of VS Code's working tree group. */
const WORKING_TREE: Record<string, GitResourceStatus> = {
  M: 'modified',
  D: 'deleted',
  A: 'intent-to-add',
  R: 'intent-to-rename',
  T: 'type-changed',
}

/** The index's letter, a resource of VS Code's index group. */
const INDEX: Record<string, GitResourceStatus> = {
  M: 'index-modified',
  A: 'index-added',
  D: 'index-deleted',
  R: 'index-renamed',
  C: 'index-copied',
}

/**
 * The status whose mark VS Code's explorer shows for a file: a conflict's, or
 * else the working tree's, or else untracked, or else the index's; `null`
 * when VS Code files nothing that marks it.
 */
export function gitResourceStatus(status: string): GitResourceStatus | null {
  const conflict = CONFLICTS[status]
  if (conflict)
    return conflict
  if (status === '??')
    return 'untracked'
  return WORKING_TREE[status.charAt(1)] ?? INDEX[status.charAt(0)] ?? null
}

/** Which of VS Code's git decoration colors a status wears. */
export type GitMarkColor = 'added' | 'modified' | 'deleted' | 'renamed' | 'untracked' | 'conflicting'

export interface GitMark {
  letter: string
  /** VS Code's words for the status, in the page's language: its tooltip. */
  text: string
  color: GitMarkColor
  /** VS Code strikes a deleted file's name through. */
  strike: boolean
}

/** A status as the catalogue words it, `gitStatus.<key>`. */
type GitMarkKey =
  | 'indexModified' | 'indexAdded' | 'indexDeleted' | 'indexRenamed' | 'indexCopied'
  | 'modified' | 'deleted' | 'untracked' | 'intentToAdd' | 'intentToRename' | 'typeChanged'
  | 'bothDeleted' | 'addedByUs' | 'deletedByThem' | 'addedByThem' | 'deletedByUs' | 'bothAdded' | 'bothModified'
  | 'added' | 'renamed'

type GitMarkEntry = Omit<GitMark, 'text'> & { key: GitMarkKey }

export const GIT_MARKS: Record<GitResourceStatus, GitMarkEntry> = {
  'index-modified': { letter: 'M', key: 'indexModified', color: 'modified', strike: false },
  'index-added': { letter: 'A', key: 'indexAdded', color: 'added', strike: false },
  'index-deleted': { letter: 'D', key: 'indexDeleted', color: 'deleted', strike: true },
  'index-renamed': { letter: 'R', key: 'indexRenamed', color: 'renamed', strike: false },
  'index-copied': { letter: 'C', key: 'indexCopied', color: 'renamed', strike: false },
  modified: { letter: 'M', key: 'modified', color: 'modified', strike: false },
  deleted: { letter: 'D', key: 'deleted', color: 'deleted', strike: true },
  untracked: { letter: 'U', key: 'untracked', color: 'untracked', strike: false },
  'intent-to-add': { letter: 'A', key: 'intentToAdd', color: 'added', strike: false },
  'intent-to-rename': { letter: 'R', key: 'intentToRename', color: 'renamed', strike: false },
  'type-changed': { letter: 'T', key: 'typeChanged', color: 'modified', strike: false },
  'both-deleted': { letter: '!', key: 'bothDeleted', color: 'conflicting', strike: true },
  'added-by-us': { letter: '!', key: 'addedByUs', color: 'conflicting', strike: false },
  'deleted-by-them': { letter: '!', key: 'deletedByThem', color: 'conflicting', strike: true },
  'added-by-them': { letter: '!', key: 'addedByThem', color: 'conflicting', strike: false },
  'deleted-by-us': { letter: '!', key: 'deletedByUs', color: 'conflicting', strike: true },
  'both-added': { letter: '!', key: 'bothAdded', color: 'conflicting', strike: false },
  'both-modified': { letter: '!', key: 'bothModified', color: 'conflicting', strike: false },
}

/** The marks of a commit's files, by the one letter git's log gives each, in VS Code's colors. */
const COMMIT_MARKS: Record<string, GitMarkEntry> = {
  A: { letter: 'A', key: 'added', color: 'added', strike: false },
  M: { letter: 'M', key: 'modified', color: 'modified', strike: false },
  D: { letter: 'D', key: 'deleted', color: 'deleted', strike: true },
  R: { letter: 'R', key: 'renamed', color: 'renamed', strike: false },
}

/**
 * How VS Code marks a file with git's two letters, or a commit's file with
 * the one letter git's log gives it; `null` when it does not.
 */
export function gitMark(status: string): GitMark | null {
  const resource = status.length === 1 ? null : gitResourceStatus(status)
  const entry = resource === null ? COMMIT_MARKS[status] : GIT_MARKS[resource]
  if (!entry)
    return null
  const { key, ...mark } = entry
  return { ...mark, text: t(`gitStatus.${key}`) }
}
