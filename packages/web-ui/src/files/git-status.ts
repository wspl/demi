/**
 * How VS Code's Git marks a file, from git's two status letters for it (the
 * `XY` of `git status --porcelain`: the index against HEAD, then the working
 * tree against the index). VS Code files a path under one or two of its
 * resource groups, each with a status; its explorer then lets a later group's
 * mark replace an earlier one's, in the order index, untracked, working tree,
 * conflicts. So a staged new file edited again (`AM`) shows the working
 * tree's M, and a staged type change alone (`T `) shows nothing, since VS Code
 * files no index resource for it. The words and letters are VS Code's own.
 */

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
  /** VS Code's words for the status, its tooltip. */
  text: string
  color: GitMarkColor
  /** VS Code strikes a deleted file's name through. */
  strike: boolean
}

export const GIT_MARKS: Record<GitResourceStatus, GitMark> = {
  'index-modified': { letter: 'M', text: 'Index Modified', color: 'modified', strike: false },
  'index-added': { letter: 'A', text: 'Index Added', color: 'added', strike: false },
  'index-deleted': { letter: 'D', text: 'Index Deleted', color: 'deleted', strike: true },
  'index-renamed': { letter: 'R', text: 'Index Renamed', color: 'renamed', strike: false },
  'index-copied': { letter: 'C', text: 'Index Copied', color: 'renamed', strike: false },
  modified: { letter: 'M', text: 'Modified', color: 'modified', strike: false },
  deleted: { letter: 'D', text: 'Deleted', color: 'deleted', strike: true },
  untracked: { letter: 'U', text: 'Untracked', color: 'untracked', strike: false },
  'intent-to-add': { letter: 'A', text: 'Intent to Add', color: 'added', strike: false },
  'intent-to-rename': { letter: 'R', text: 'Intent to Rename', color: 'renamed', strike: false },
  'type-changed': { letter: 'T', text: 'Type Changed', color: 'modified', strike: false },
  'both-deleted': { letter: '!', text: 'Conflict: Both Deleted', color: 'conflicting', strike: true },
  'added-by-us': { letter: '!', text: 'Conflict: Added By Us', color: 'conflicting', strike: false },
  'deleted-by-them': { letter: '!', text: 'Conflict: Deleted By Them', color: 'conflicting', strike: true },
  'added-by-them': { letter: '!', text: 'Conflict: Added By Them', color: 'conflicting', strike: false },
  'deleted-by-us': { letter: '!', text: 'Conflict: Deleted By Us', color: 'conflicting', strike: true },
  'both-added': { letter: '!', text: 'Conflict: Both Added', color: 'conflicting', strike: false },
  'both-modified': { letter: '!', text: 'Conflict: Both Modified', color: 'conflicting', strike: false },
}

/** How VS Code marks a file with git's two letters; `null` when it does not. */
export function gitMark(status: string): GitMark | null {
  const resource = gitResourceStatus(status)
  return resource ? GIT_MARKS[resource] : null
}
