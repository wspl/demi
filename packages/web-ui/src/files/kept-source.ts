/**
 * A file source that keeps what it read (`plugin-pages.md` § What the
 * service keeps): the reads a host makes, each a request, over the Host's
 * kept files (`HostFiles`). A view shows an entry through the source and
 * sees it replaced in place when it is read again; a one-shot read answers
 * from what is confirmed, and reads otherwise. The source's own writes edit
 * the listings they change as their answers tell, without listing them
 * again: the Host's watch reports the write, and the folder is listed once
 * for its reports (`web-application.md` § Requests for one action).
 */
import { reactive } from 'vue'
import type { HostFiles, KeptSpec, Showing } from './file-cache'
import { baseName } from '@demicodes/utils'
import { parentPath } from './paths'
import type { ChangeSetSource, ChangeSides, WorkingTreeChange } from './changes'
import type {
  FileBrowserEntry,
  FileBrowserSource,
  FileContents,
  FileDescription,
  FileText,
  FileUploadOptions,
  FileWatchNote,
} from './types'

/** A file's bytes as a host serves them: where they load from, and their description, each a request. */
export interface ContentReads {
  url: FileContents['url']
  describe(path: string): Promise<FileDescription>
  readStart?: FileContents['readStart']
}

/**
 * How a host reads and writes a Host's files, each a request: the source
 * built over them keeps what they answer.
 */
export interface FileReads extends Pick<FileBrowserSource, 'platform' | 'home' | 'createDirectory' | 'upload' | 'remove'> {
  list(path: string): Promise<FileBrowserEntry[]>
  /**
   * A file's text with its version; null when the file still has the
   * version `held` names, so its text did not travel.
   */
  readText?(path: string, held: string | null): Promise<FileText | null>
  contents?: ContentReads
}

/**
 * Whoever follows what a source shows on the Host, such as a conversation's
 * file watch: each path shown is told until its view lets it go.
 */
export interface FileFollower {
  /** A view shows an entry at `path`; the answer ends that. */
  show(path: string): () => void
  /** What the follower says of its watch, for the views. */
  readonly note: FileWatchNote | null
}

export interface KeptSourceOptions {
  files: HostFiles
  follower?: FileFollower
}

/** A view's hold that also tells the follower. */
function followed<T>(showing: Showing<T>, path: string, follower: FileFollower | undefined): Showing<T> {
  const unfollow = follower?.show(path)
  return {
    entry: showing.entry,
    retry: showing.retry,
    release() {
      showing.release()
      unfollow?.()
    },
  }
}

function listingSpec(reads: FileReads, path: string): KeptSpec<FileBrowserEntry[]> {
  return {
    kind: 'listing',
    path,
    read: () => reads.list(path),
    // Its names are its text.
    size: (entries) => entries.reduce((sum, entry) => sum + entry.name.length, 0),
  }
}

function textSpec(readText: NonNullable<FileReads['readText']>, path: string): KeptSpec<FileText> {
  return {
    kind: 'text',
    path,
    async read(held) {
      const read = await readText(path, held?.version ?? null)
      // Unchanged: what is kept is the file's text.
      return read ?? held!
    },
    size: (text) => text.text.length,
  }
}

/** The contents of `reads` over the kept files, their descriptions kept as `kind`. */
export function keptContents(
  reads: ContentReads,
  { files, follower }: KeptSourceOptions,
  kind: 'description' | 'committed' = 'description',
  absolute: (path: string) => string = (path) => path,
): FileContents {
  const spec = (path: string): KeptSpec<FileDescription> => ({
    kind,
    path: absolute(path),
    key: path,
    read: () => reads.describe(path),
    size: () => 0,
  })
  return {
    url: reads.url,
    describe: (path) => files.get(spec(path)),
    showDescription: (path) => followed(files.show(spec(path)), absolute(path), follower),
    // A part of a file is read for the view that shows it, never kept.
    ...(reads.readStart ? { readStart: reads.readStart } : {}),
  }
}

/** A file source over `reads` that keeps what it read in `files`. */
export function keptSource(reads: FileReads, options: KeptSourceOptions): FileBrowserSource {
  const { files, follower } = options
  const listing = (path: string) => listingSpec(reads, path)
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
    get watch() {
      return follower?.note ?? null
    },
    list: (path) => files.get(listingSpec(reads, path)),
    showListing: (path) => followed(files.show(listingSpec(reads, path)), path, follower),
    ...(readText
      ? {
          read: async (path: string) => (await files.get(textSpec(readText, path))).text,
          showText: (path: string) => followed(files.show(textSpec(readText, path)), path, follower),
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

/** The working tree's uncommitted changes as a host reads them, each a request. */
export interface ChangeReads {
  /** The list of changes, or why there is none. */
  list(): Promise<{ files: WorkingTreeChange[]; truncated: boolean; repository: boolean }>
  /** Both sides of one changed file, by its path relative to the working tree. */
  sides(path: string): Promise<ChangeSides>
  /** The committed side's bytes, by path relative to the working tree. */
  committed?: ContentReads
}

/** The kept changes list and what a view shows of it. */
interface ChangesList {
  files: WorkingTreeChange[]
  truncated: boolean
  repository: boolean
}

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
  const { files, follower } = options
  const absolute = (path: string) => `${root.replace(/\/$/, '')}/${path}`
  const listSpec: KeptSpec<ChangesList> = {
    kind: 'changes',
    path: root,
    read: () => reads.list(),
    size: (list) => list.files.reduce((sum, file) => sum + file.path.length, 0),
    outsideRepository: (list) => !list.repository,
  }
  const sidesSpec = (path: string): KeptSpec<ChangeSides> => ({
    kind: 'sides',
    path: absolute(path),
    key: root,
    read: () => reads.sides(path),
    size: (sides) => sides.original.length + sides.modified.length,
  })
  const entry = () => files.peek(listSpec)
  const source = reactive({
    get watch() {
      return follower?.note ?? null
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
      return failure ? failure.message ?? 'The changes could not be listed.' : null
    },
    refresh() {
      files.retry(listSpec)
    },
    showSides: (path: string) => followed(files.show(sidesSpec(path)), absolute(path), follower),
    ...(reads.committed ? { committed: keptContents(reads.committed, options, 'committed', absolute) } : {}),
    show: () => followed(files.show(listSpec), root, follower),
  })
  return source
}
