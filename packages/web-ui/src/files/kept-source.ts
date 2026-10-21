/**
 * A file source that keeps what it read: the reads a host makes, each a
 * request, over the Host's kept files (`HostFiles`), whose entries the source's
 * watch keeps current.
 *
 * - A view shows an entry through the source and sees it replaced in place when
 *   it is read again.
 * - A one-shot read answers from what is current, and reads otherwise.
 * - The source's own writes edit the listings they change as their answers
 *   tell, without listing them again: the Host's watch reports the write, and
 *   the folder is listed once for it.
 */
import { reactive } from 'vue'
import type { Answered, HostFiles, KeptSpec, Read, Showing } from './file-cache'
import { baseName } from '@demicodes/utils'
import { t } from './locales/text'
import { parentPath } from './paths'
import type { ChangeSetSource, ChangeSides, WorkingTreeChange } from './changes'
import type { BranchList, CommitDetails, CommitHistorySource, CommitLogPage } from './history'
import type { WatchChannel } from './watch-channel'
import type {
  FileBrowserEntry,
  FileBrowserSource,
  FileContents,
  FileDescription,
  FileText,
  FileUploadOptions,
} from './types'

/** A file's bytes as a host serves them: where they load from, and their description, each a request. */
export interface ContentReads {
  url: FileContents['url']
  describe(path: string): Promise<Answered<FileDescription>>
  readStart?: FileContents['readStart']
}

/**
 * How a host reads and writes a Host's files, each a request: the source
 * built over them keeps what they answer, each with the Host's number
 * before it read.
 */
export interface FileReads extends Pick<FileBrowserSource, 'platform' | 'home' | 'createDirectory' | 'upload' | 'remove'> {
  list(path: string): Promise<Answered<FileBrowserEntry[]>>
  /**
   * A file's text with its version; unchanged when the file still has the
   * version `held` names, so its text did not travel. A file that is not
   * text, or too large to read as text, rejects with that answer.
   */
  readText?(path: string, held: string | null): Promise<Read<FileText>>
  contents?: ContentReads
}

export interface KeptSourceOptions {
  files: HostFiles
  /** The watch of the source's route, which carries the subscriptions of what it keeps; none without one. */
  watch?: WatchChannel | null
  /** The source's views show sizes and times, as a folder dialog does, so its listings follow them. */
  details?: boolean
}

function listingSpec(reads: FileReads, path: string, { watch, details }: KeptSourceOptions): KeptSpec<FileBrowserEntry[]> {
  return {
    kind: 'listing',
    path,
    details: details ?? false,
    watch: watch ?? null,
    read: () => reads.list(path),
    // Its names are its text.
    size: (entries) => entries.reduce((sum, entry) => sum + entry.name.length, 0),
  }
}

function textSpec(readText: NonNullable<FileReads['readText']>, path: string, watch: WatchChannel | null): KeptSpec<FileText> {
  return {
    kind: 'text',
    path,
    watch,
    read: (held) => readText(path, held),
    size: (text) => text.text.length,
    version: (text) => text.version,
  }
}

/** The contents of `reads` over the kept files, their descriptions kept as `kind`. */
export function keptContents(
  reads: ContentReads,
  { files, watch }: KeptSourceOptions,
  kind: 'description' | 'committed' = 'description',
  absolute: (path: string) => string = (path) => path,
  root?: string,
  /** Tells apart the committed contents of one commit from another's and the last commit's. */
  commit?: string,
): FileContents {
  const spec = (path: string): KeptSpec<FileDescription> => ({
    kind,
    path: absolute(path),
    key: commit === undefined ? path : `${commit}\u0000${path}`,
    ...(root === undefined ? {} : { root }),
    watch: watch ?? null,
    read: () => reads.describe(path),
    size: () => 0,
  })
  return {
    url: reads.url,
    describe: (path) => files.get(spec(path)),
    showDescription: (path) => files.show(spec(path)),
    offerDescription: (path, answered) => files.offer(spec(path), answered),
    // A part of a file is read for the view that shows it, never kept.
    ...(reads.readStart ? { readStart: reads.readStart } : {}),
  }
}

/** A file source over `reads` that keeps what it read in `files`. */
export function keptSource(reads: FileReads, options: KeptSourceOptions): FileBrowserSource {
  const { files } = options
  const watch = options.watch ?? null
  const listing = (path: string) => listingSpec(reads, path, options)
  /**
   * `entry` is in its folder `directory` now, in place of what had its name,
   * and each folder above it is in its own: a write makes the folders it
   * needs, and a folder listed already keeps what was listed of it.
   */
  const added = (directory: string, entry: FileBrowserEntry) => {
    files.amend(listing(directory), (entries) => [...entries.filter((kept) => kept.name !== entry.name), entry])
    for (let folder = directory; parentPath(folder) !== folder; folder = parentPath(folder)) {
      const made: FileBrowserEntry = { name: baseName(folder), isDirectory: true }
      files.amend(listing(parentPath(folder)), (entries) =>
        entries.some((kept) => kept.name === made.name) ? entries : [...entries, made])
    }
  }
  /** Nothing is at `path` now. */
  const removed = (path: string) => {
    files.amend(listing(parentPath(path)), (entries) => entries.filter((kept) => kept.name !== baseName(path)))
  }
  const readText = reads.readText
  return {
    platform: reads.platform,
    home: reads.home,
    list: (path) => files.get(listing(path)),
    showListing: (path) => files.show(listing(path)),
    ...(readText
      ? {
          read: async (path: string) => (await files.get(textSpec(readText, path, watch))).text,
          showText: (path: string) => files.show(textSpec(readText, path, watch)),
        }
      : {}),
    ...(reads.contents ? { contents: keptContents(reads.contents, options) } : {}),
    ...(reads.createDirectory
      ? {
          createDirectory: async (path: string, signal?: AbortSignal) => {
            await reads.createDirectory!(path, signal)
            added(parentPath(path), { name: baseName(path), isDirectory: true })
          },
        }
      : {}),
    ...(reads.upload
      ? {
          // The tree adds the entry with the name and size it sent.
          upload: async (path: string, file: File, options: FileUploadOptions) => {
            await reads.upload!(path, file, options)
            added(parentPath(path), { name: baseName(path), isDirectory: false, size: file.size })
          },
        }
      : {}),
    ...(reads.remove
      ? {
          remove: async (path: string, signal?: AbortSignal) => {
            await reads.remove!(path, signal)
            removed(path)
          },
        }
      : {}),
  }
}

/**
 * The working tree's uncommitted changes as a host reads them, each a request with
 * the Host's number before it read.
 *
 * Each read names the version it holds, `held`, and answers unchanged when the
 * Host's answer is still that one.
 */
export interface ChangeReads {
  /** The list of changes, or why there is none. */
  list(held: string | null): Promise<Read<ChangesList>>
  /**
   * Both sides of one changed file, by its path relative to the working
   * tree, the old side at `from` for a renamed file; a side that is not
   * text rejects with that answer.
   */
  sides(path: string, held: string | null, from?: string): Promise<Read<KeptSides>>
  /** The committed side's bytes, by path relative to the working tree. */
  committed?: ContentReads
}

/** The kept changes list and what a view shows of it, with the version it was read at. */
export interface ChangesList {
  files: WorkingTreeChange[]
  truncated: boolean
  repository: boolean
  version: string | null
}

/** A changed file's two sides as kept, with the version they were read at. */
export type KeptSides = ChangeSides & { version: string | null }

/**
 * The working tree under `root` as a change set over the kept files: the
 * list, each file's sides and committed contents, kept like the files.
 * `show` follows the list for a component until its scope ends.
 */
export function keptChangeSet(
  reads: ChangeReads,
  root: string,
  options: KeptSourceOptions,
): ChangeSetSource & { refresh(): void; show(): Showing<ChangesList> } {
  const { files } = options
  const watch = options.watch ?? null
  const absolute = (path: string) => `${root.replace(/\/$/, '')}/${path}`
  // Each is read again with the version held, and an unchanged answer
  // keeps what is held, so a view shows nothing new.
  const listSpec: KeptSpec<ChangesList> = {
    kind: 'changes',
    path: root,
    watch,
    read: (held) => reads.list(held),
    size: (list) => list.files.reduce((sum, file) => sum + file.path.length, 0),
    version: (list) => list.version,
  }
  const sidesSpec = (path: string, from?: string): KeptSpec<KeptSides> => ({
    kind: 'sides',
    path: absolute(path),
    key: from === undefined ? root : `${root}\u0000${from}`,
    root,
    watch,
    read: (held) => reads.sides(path, held, from),
    size: (sides) => sides.original.length + sides.modified.length,
    version: (sides) => sides.version,
  })
  const entry = () => files.peek(listSpec)
  const source = reactive({
    get unwatched() {
      return entry().unwatched
    },
    get files() {
      return entry().value?.files ?? []
    },
    get truncated() {
      return entry().value?.truncated ?? false
    },
    get unavailable() {
      const value = entry().value
      return value && !value.repository ? 'no-repository' as const : null
    },
    get refreshing() {
      return entry().reading
    },
    get failure() {
      const failure = entry().failure
      return failure ? failure.message ?? t('changes.listFailed') : null
    },
    refresh() {
      files.retry(listSpec)
    },
    showSides: (path: string, from?: string) => files.show(sidesSpec(path, from)),
    ...(reads.committed ? { committed: keptContents(reads.committed, options, 'committed', absolute, root) } : {}),
    show: () => files.show(listSpec),
  })
  return source
}

/**
 * A repository's history as a host reads it, each a request with the Host's
 * number before it read: an answer that nothing is there, a branch deleted
 * or a commit the repository lacks, rejects as not found.
 */
export interface HistoryReads {
  branches(): Promise<Answered<BranchList>>
  log(branch: string | null, after: string): Promise<Answered<CommitLogPage>>
  commit(commit: string): Promise<Answered<CommitDetails>>
  sides(commit: string, path: string, from?: string): Promise<Answered<ChangeSides>>
  /** A commit's files' bytes, by path relative to the working tree's root. */
  contents?(commit: string): ContentReads
}

/** A commit's full id, SHA-1's or SHA-256's, as against an abbreviation, which a new commit can make ambiguous. */
const FULL_ID = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/

/**
 * The history of the repository the working tree under `root` is in, kept like
 * its files.
 *
 * - Each part follows the repository's `head`, which a commit, a checkout or a
 *   new branch changes, so a view shows it again at once
 *   (`packages/web-ui/src/files/history.ts`).
 * - A file's sides at a commit's full id never change, and are read once.
 * - Outside a repository, the `head` it follows changes when one is made there.
 */
export function keptHistory(reads: HistoryReads, root: string, options: KeptSourceOptions): CommitHistorySource {
  const { files } = options
  const watch = options.watch ?? null
  const head = () => [{ kind: 'head' as const, path: root }]
  const answer = <T>(key: string, read: () => Promise<Answered<T>>, size: (value: T) => number, immutable = false): KeptSpec<T> => ({
    kind: 'answer',
    path: root,
    key,
    watch,
    read,
    size,
    // What a commit's full id names never changes: nothing moves it.
    ...(immutable ? { immutable, subjects: () => [] } : { subjects: head }),
  })
  const messages = (commits: readonly { message: string }[]) => commits.reduce((sum, commit) => sum + commit.message.length, 0)
  const absolute = (path: string) => `${root.replace(/\/$/, '')}/${path}`
  const contents = new Map<string, FileContents>()
  return {
    showBranches: () => files.show(answer('branches', () => reads.branches(), (list) => list.branches.length)),
    showLog: (branch, after) => files.show(answer(
      `log\u0000${branch ?? ''}\u0000${after}`,
      () => reads.log(branch, after),
      (page) => messages(page.commits),
    )),
    showCommit: (commit) => files.show(answer(
      `commit\u0000${commit}`,
      () => reads.commit(commit),
      (details) => details.commit.message.length + details.files.reduce((sum, file) => sum + file.path.length, 0),
    )),
    showSides: (commit, path, from) => files.show(answer(
      `sides\u0000${commit}\u0000${path}\u0000${from ?? ''}`,
      () => reads.sides(commit, path, from),
      (sides) => sides.original.length + sides.modified.length,
      FULL_ID.test(commit),
    )),
    ...(reads.contents
      ? {
          contents: (commit: string) => {
            let kept = contents.get(commit)
            if (!kept) {
              kept = keptContents(reads.contents!(commit), options, 'committed', absolute, root, commit)
              contents.set(commit, kept)
            }
            return kept
          },
        }
      : {}),
  }
}
