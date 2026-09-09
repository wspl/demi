/**
 * Contracts of the file browser. A host maps its own directory source (the Web API's
 * device fs, a prototype's in-memory tree) onto these; the browser owns navigation,
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

/** Why a directory could not be listed. `kind` picks the empty-state copy; `message` is shown as the detail. */
export interface FileBrowserFailure {
  kind: 'not-found' | 'permission' | 'offline' | 'other'
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

/** The operating system behind a source; picks the root glyph. */
export type FileBrowserPlatform = 'macos' | 'linux' | 'windows'

/** The directory tree behind one browser: where it starts, what runs it and how it reads. */
export interface FileBrowserSource {
  platform: FileBrowserPlatform
  /** The directory the browser opens in when the caller names none, and where Home goes. */
  home: string
  /** Lists one directory. Rejects with a `FileBrowserError` for a known failure; anything else reads as `other`. */
  list(path: string, signal?: AbortSignal): Promise<FileBrowserEntry[]>
  /** Absent when the source cannot create directories; the browser then offers no New folder. */
  createDirectory?(path: string, signal?: AbortSignal): Promise<void>
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
  icon?: Component
}

/** What the browser is for: the confirm button and what a click on a row does follow from it. */
export type FileBrowserMode = 'file' | 'directory'
