/**
 * Contracts of the file browser. A host maps its own directory source (the Web API's
 * file endpoints, a prototype's in-memory tree) onto these; the file browser owns navigation,
 * selection and presentation and never knows where the entries come from.
 *
 * Paths are POSIX: `/` separates segments and `/` is the root. A source for another
 * convention translates at its boundary.
 */
import type { Component } from 'vue'
import type { Showing } from './file-cache'
import type { SentenceText, TitleText } from '../ui/ui-text'
import type { DeviceState } from '../devices/state'

export type { Showing, ShownEntry } from './file-cache'

/** One row of a directory listing. Size and time are shown when the source knows them. */
export interface FileBrowserEntry {
  name: string
  isDirectory: boolean
  /** Bytes; files only. */
  size?: number
  /** ISO timestamp of the last modification. */
  modifiedAt?: string
}

/**
 * Why a directory could not be listed, a file read or written. `kind` picks
 * the copy; `message` is shown as the detail. `binary` and `too-large` are
 * files the text read cannot show, which a view shows as a card instead;
 * `exists` is a file an upload was not to replace.
 */
export interface FileBrowserFailure {
  kind: 'not-found' | 'permission' | 'offline' | 'binary' | 'too-large' | 'exists' | 'other'
  message?: SentenceText
}

export class FileBrowserError extends Error {
  readonly kind: FileBrowserFailure['kind']

  constructor(kind: FileBrowserFailure['kind'], message?: SentenceText) {
    super(message ?? kind)
    this.name = 'FileBrowserError'
    this.kind = kind
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

/** A file's bytes as the page loads them (`file-previews.md` § Getting the bytes). */
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
}

/** A file's text and the version it was read at; null when its source has no versions. */
export interface FileText {
  text: string
  version: string | null
}

/**
 * What a source says of the Host's watch of what it shows (`plugin-pages.md`
 * § What the service keeps); reactive.
 */
export interface FileWatchNote {
  /** Why the Host cannot watch; null while it can, or no watch says. The views then show files as last read. */
  readonly unavailable: string | null
  /** Reads everything shown again now. */
  refresh(): void
}

/** The operating system behind a source; picks the root glyph. */
export type FileBrowserPlatform = 'macos' | 'linux' | 'windows'

/**
 * The directory tree behind one file browser: where it starts, what runs it
 * and how it reads. It keeps what it read (`keptSource`): a view shows an
 * entry with `showListing`, `showText` or `showDescription` and sees it
 * replaced in place when it is read again; a one-shot read answers from what
 * is kept while the Host confirms it.
 */
export interface FileBrowserSource {
  platform: FileBrowserPlatform
  /** The directory the file browser opens in when the caller names none, and where Home goes. */
  home: string
  /** What the source says of the Host's watch; null without one. */
  readonly watch: FileWatchNote | null
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
   * directories above it that are missing: `progress` hears how many have
   * gone, and aborting `signal` stops the upload and leaves `path` as it
   * was. A `path` already taken rejects with a `FileBrowserError` of kind
   * `exists` unless `replace`; a directory there is never replaced. Absent
   * when the source cannot write; the tree then offers no Upload.
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
  icon?: Component
}

/** What the file browser is for: the confirm button and what a click on a row does follow from it. */
export type FileBrowserMode = 'file' | 'directory'
