/**
 * Contracts of the file browser.
 *
 * A host maps its own directory source (the Web API's file endpoints, a
 * prototype's in-memory tree) onto these. The file browser owns navigation,
 * selection and presentation and never knows where the entries come from.
 *
 * Paths are POSIX: `/` separates segments and `/` is the root. A source for
 * another convention translates at its boundary.
 */
import type { Showing } from './file-cache'
import type { ReadAt } from './watch-channel'
import type { SentenceText, TitleText } from '../ui/ui-text'
import type { DeviceMark } from '../devices/mark'
import type { DeviceState } from '../devices/state'
import type { HostKind } from '../hosts/types'

export type { Showing, ShownEntry } from './file-cache'

/**
 * One row of a directory listing. Size and time are shown when the source
 * knows them. A symbolic link is what its target is: a link to a directory
 * opens as a folder, its entries the target's, and any other link as a file.
 */
export interface FileBrowserEntry {
  name: string
  /** Whether it opens as a folder: a directory, or a link to one. */
  isDirectory: boolean
  /** Bytes; files only. A link's are its own, as `lstat` reports them. */
  size?: number
  /** ISO timestamp of the last modification. */
  modifiedAt?: string
  /** Where a symbolic link points; absent for any other entry. */
  link?: FileBrowserLink
}

/** A symbolic link's target as written, and why it cannot be reached, as of its listing. */
export interface FileBrowserLink {
  target: string
  /** Absent while the target can be reached. */
  broken?: FileBrowserLinkBreak
}

/**
 * Why a link's target cannot be reached: nothing is there, or something may be
 * but cannot be read, for the system's `reason`, such as a folder on the way
 * closed to the user.
 */
export type FileBrowserLinkBreak = { kind: 'missing' } | { kind: 'unreadable'; reason: string }

/**
 * Why a directory could not be listed, a file read or written.
 *
 * `kind` picks the copy; `message` is shown as the detail.
 *
 * - `exists`: a file an upload was not to replace.
 * - `not-repository`: a history read of a folder outside any Git repository.
 * - `not-found`, `binary` and `too-large`: what the Host answered of the path,
 *   respectively nothing there, a file that is not text, and one too large to
 *   read as text. The kept files keep each as an answer, never as a failure.
 */
export interface FileBrowserFailure {
  kind: 'not-found' | 'permission' | 'offline' | 'binary' | 'too-large' | 'exists' | 'not-repository' | 'other'
  message?: SentenceText
  /** The request that failed, which the view shows with the failure. */
  requestId?: string | null
}

/** What a refusal that is the Host's answer carries: the file's version, and the Host's number before it read. */
export interface AnswerFacts {
  version?: string | null
  readAt?: ReadAt | null
  /** The request the Host's refusal answered. */
  requestId?: string | null
}

export class FileBrowserError extends Error {
  readonly kind: FileBrowserFailure['kind']
  /** The version of the file the Host answered of; null when it named none. */
  readonly version: string | null
  /** The Host's number before it read; null when it named none. */
  readonly readAt: ReadAt | null
  /** The request that was refused, which the page shows with the failure. */
  readonly requestId: string | null

  constructor(kind: FileBrowserFailure['kind'], message?: SentenceText, facts: AnswerFacts = {}) {
    super(message ?? kind)
    this.name = 'FileBrowserError'
    this.kind = kind
    this.version = facts.version ?? null
    this.readAt = facts.readAt ?? null
    this.requestId = facts.requestId ?? null
  }
}

/** A file's size, modification time and version, read without its bytes. */
export interface FileDescription {
  size: number
  /** ISO time; null when the source does not know it, as for a committed version. */
  modifiedAt: string | null
  /** What `FileContents.url` pins a preview to; null when the source has none. */
  version: string | null
}

/** A file's bytes as the page loads them. */
export interface FileContents {
  /**
   * Where the page loads the file from: `version` pins the one a preview
   * opened, and `download` asks for it as an attachment.
   */
  url(path: string, options?: { version?: string; download?: boolean }): string
  /** The file's description once, as kept or read. */
  describe(path: string): Promise<FileDescription>
  /** Shows the file's description until released, kept current as its source keeps it. */
  showDescription(path: string): Showing<FileDescription>
  /**
   * The file's first `length` bytes, or all of a shorter file, at `version`
   * when given; absent when the source reads no part of a file. A text file
   * too large to read whole shows them.
   */
  readStart?(path: string, length: number, options?: { version?: string; signal?: AbortSignal }): Promise<Uint8Array>
}

/** A file's text and the version it was read at; null when its source has no versions. */
export interface FileText {
  text: string
  version: string | null
}

/** The operating system behind a source; picks the root glyph. */
export type FileBrowserPlatform = 'macos' | 'linux' | 'windows'

/**
 * The directory tree behind one file browser: where it starts, what runs it and how
 * it reads.
 *
 * It keeps what it read (`keptSource`). A view shows an entry with `showListing`,
 * `showText` or `showDescription` and sees it replaced in place when it is read
 * again. A one-shot read answers from what is kept while the Host confirms it.
 */
export interface FileBrowserSource {
  platform: FileBrowserPlatform
  /** The directory the file browser opens in when the caller names none, and where Home goes. */
  home: string
  /** Lists one directory once. Rejects with a `FileBrowserError` for a known failure; anything else reads as `other`. */
  list(path: string): Promise<FileBrowserEntry[]>
  /** Shows one directory's listing until released. */
  showListing(path: string): Showing<FileBrowserEntry[]>
  /**
   * Makes the directory at `path` and any missing above it; one already
   * there is left as it is. Absent when the source cannot create
   * directories; the file browser then offers no New folder.
   */
  createDirectory?(path: string, signal?: AbortSignal): Promise<void>
  /** Reads one file as text once. Absent when the source cannot read files; a file view then has nothing to show. */
  read?(path: string): Promise<string>
  /** Shows one file's text until released; present with `read`. */
  showText?(path: string): Showing<FileText>
  /** The files' bytes for previews and Download; absent when the source serves none. */
  contents?: FileContents
  /**
   * Writes `file` to `path` whole, its bytes streamed as they go, making the
   * directories above it that are missing.
   *
   * - `progress` hears how many bytes have gone.
   * - Aborting `signal` stops the upload and leaves `path` as it was.
   * - A `path` already taken rejects with a `FileBrowserError` of kind `exists`
   *   unless `replace`; a directory there is never replaced.
   *
   * Absent when the source cannot write; the tree then offers no Upload.
   */
  upload?(path: string, file: File, options: FileUploadOptions): Promise<void>
  /**
   * Deletes the file or the directory at `path`, a directory with everything
   * in it; nothing there is fine. Absent when the source cannot delete; an
   * upload then cannot replace a folder.
   */
  remove?(path: string, signal?: AbortSignal): Promise<void>
}

export interface FileUploadOptions {
  replace: boolean
  signal: AbortSignal
  progress(sent: number): void
}

/** A shortcut in the sidebar: a home, a recent workspace, a pinned directory. */
export interface FileBrowserPlace {
  path: string
  /** Defaults to the last segment of the path. */
  label?: string
  /** A Material Icon Theme id in place of the one the path's name resolves to, e.g. `folder-home`. */
  icon?: string
}

/** Sidebar shortcuts under a caption, the way Windows groups Quick access and This PC. */
export interface FileBrowserPlaceGroup {
  label?: TitleText
  places: FileBrowserPlace[]
}

/** A machine the sidebar can switch the file browser to. The caller swaps the source when one is chosen. */
export interface FileBrowserHost {
  id: string
  label: string
  state: DeviceState
  /** A stopped host can be selected when its source wakes it on access. */
  canWake?: boolean
  /** Which kind of host it is, for its glyph; a device when omitted. */
  kind?: HostKind
  /** A device's mark, with the number it shows (`shownMark`); none for the Cloud. */
  mark?: DeviceMark | null
}

/** What the file browser is for: the confirm button and what a click on a row does follow from it. */
export type FileBrowserMode = 'file' | 'directory'
