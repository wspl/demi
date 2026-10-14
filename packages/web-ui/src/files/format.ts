import type { SentenceText } from '../ui/ui-text'
import type { FileBrowserLink } from './types'

/**
 * What a symbolic link's mark says (`file-previews.md` § Symbolic links):
 * `Symbolic link to ../shared/vendor`; of a target that is not there
 * `Symbolic link to /tmp/gone.log, which does not exist`; and of one that
 * cannot be read, the system's reason as a clause of the sentence,
 * `Symbolic link to /root/notes, which cannot be read: permission denied`.
 */
export function linkDescription(link: FileBrowserLink): SentenceText {
  const named = `Symbolic link to ${link.target}`
  switch (link.broken?.kind) {
    case undefined:
      return named
    case 'missing':
      return `${named}, which does not exist`
    case 'unreadable': {
      // The system words its reason as a sentence of its own: `Permission denied`, `Access is denied.`
      const reason = link.broken.reason.replace(/\.$/, '')
      return `${named}, which cannot be read: ${reason.charAt(0).toLowerCase()}${reason.slice(1)}`
    }
  }
}

/** `1.2 MB`, `840 KB`, `12 B`. */
export function formatBytes(size: number): string {
  if (size < 1024)
    return `${size} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = size / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`
}

/** A name's extension in capitals, `TS` for `index.ts`; null for a name without one, such as `Makefile` or `.zshrc`. */
export function fileExtension(name: string): string | null {
  const dot = name.lastIndexOf('.')
  if (dot <= 0 || dot === name.length - 1)
    return null
  return name.slice(dot + 1).toUpperCase()
}

/** The Type column: `Folder`, `TS file`, or `File` when the name has no extension. */
export function entryKind(name: string, isDirectory: boolean): string {
  if (isDirectory)
    return 'Folder'
  const extension = fileExtension(name)
  return extension === null ? 'File' : `${extension} file`
}
