import { pageLanguage } from '../i18n/text'
import type { SentenceText } from '../ui/ui-text'
import { t } from './locales/text'
import type { FileBrowserLink } from './types'

/**
 * What a symbolic link's mark says:
 *
 * - `Symbolic link to ../shared/vendor`.
 * - Of a target that is not there: `Symbolic link to /tmp/gone.log, which doesn’t
 *   exist`.
 * - Of one that cannot be read, the system's reason as a clause of the sentence:
 *   `Symbolic link to /root/notes, which can’t be read: permission denied`.
 */
export function linkDescription(link: FileBrowserLink): SentenceText {
  const target = link.target
  switch (link.broken?.kind) {
    case undefined:
      return t('link.target', { target })
    case 'missing':
      return t('link.missing', { target })
    case 'unreadable': {
      // The system words its reason as a sentence of its own: `Permission denied`, `Access is denied.`
      const reason = link.broken.reason.replace(/\.$/, '')
      return t('link.unreadable', { target, reason: `${reason.charAt(0).toLowerCase()}${reason.slice(1)}` })
    }
  }
}

/** `linkDescription` as a sentence of its own, where the page says it in place of a file. */
export function linkSentence(link: FileBrowserLink): SentenceText {
  return t('link.sentence', { description: linkDescription(link) })
}

/**
 * `1.2 MB`, `840 KB`, `12 B`: the figure as the page's language writes it,
 * the unit as English writes it in every language.
 */
export function formatBytes(size: number): string {
  if (size < 1024)
    return `${new Intl.NumberFormat(pageLanguage()).format(size)} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = size / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  const digits = value < 10 ? 1 : 0
  const figure = new Intl.NumberFormat(pageLanguage(), { minimumFractionDigits: digits, maximumFractionDigits: digits })
  return `${figure.format(value)} ${units[unit]}`
}

/**
 * A figure against its limit as software shows it, `1.0 / 16 GB`: the unit once
 * when both sides share it, each side's own otherwise (`463 MB / 2.0 GB`).
 */
export function formatByteRatio(current: number, limit: number): string {
  const [currentValue, currentUnit] = formatBytes(current).split(' ')
  const [limitValue, limitUnit] = formatBytes(limit).split(' ')
  return currentUnit === limitUnit
    ? `${currentValue} / ${limitValue} ${limitUnit}`
    : `${currentValue} ${currentUnit} / ${limitValue} ${limitUnit}`
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
    return t('kind.folder')
  const extension = fileExtension(name)
  return extension === null ? t('kind.file') : t('kind.extensionFile', { extension })
}
