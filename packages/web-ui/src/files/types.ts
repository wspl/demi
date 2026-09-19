/**
 * Contracts of the file browser. A host maps its own directory source (the Web API's
 * file endpoints, a prototype's in-memory tree) onto these; the browser owns navigation,
 * selection and presentation and never knows where the entries come from.
 *
 * Paths are POSIX: `/` separates segments and `/` is the root. A source for another
 * convention translates at its boundary.
 */
import type { Component } from 'vue'

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
  message?: string
}

export class FileBrowserError extends Error {
  readonly kind: FileBrowserFailure['kind']

  constructor(kind: FileBrowserFailure['kind'], message?: string) {
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
  describe(path: string, signal?: AbortSignal): Promise<FileDescription>
}

/** The operating system behind a source; picks the root glyph. */
export type FileBrowserPlatform = 'macos' | 'linux' | 'windows'

/** The directory tree behind one browser: where it starts, what runs it and how it reads. */
export interface FileBrowserSource {
  platform: FileBrowserPlatform
  /** The directory the browser opens in when the caller names none, and where Home goes. */
  home: string
  /** Lists one directory. Rejects with a `FileBrowserError` for a known failure; anything else reads as `other`. */
  list(path: string, signal?: AbortSignal): Promise<FileBrowserEntry[]>
  /**
   * Makes the directory at `path` and any missing above it; one already
   * there is left as it is. Absent when the source cannot create
   * directories; the browser then offers no New folder.
   */
  createDirectory?(path: string, signal?: AbortSignal): Promise<void>
  /** Reads one file as text. Absent when the source cannot read files; a file view then has nothing to show. */
  read?(path: string, signal?: AbortSignal): Promise<string>
  /** The files' bytes for previews and Download; absent when the source serves none. */
  contents?: FileContents
  /**
   * Writes `file` to `path` whole, its bytes streamed as they go: `progress`
   * hears how many have gone, and aborting `signal` stops the upload and
   * leaves `path` as it was. A `path` already taken rejects with a
   * `FileBrowserError` of kind `exists` unless `replace`; a directory there
   * is never replaced. Absent when the source cannot write; the tree then
   * offers no Upload.
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
  label?: string
  places: FileBrowserPlace[]
}

/** A machine the sidebar can switch the browser to. The caller swaps the source when one is chosen. */
export interface FileBrowserHost {
  id: string
  label: string
  online: boolean
  /** A stopped host can be selected when its source wakes it on access. */
  canWake?: boolean
  icon?: Component
}

/** What the browser is for: the confirm button and what a click on a row does follow from it. */
export type FileBrowserMode = 'file' | 'directory'
