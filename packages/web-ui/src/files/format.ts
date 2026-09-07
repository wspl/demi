import dayjs from 'dayjs'

/** `1.2 MB`, `840 KB`, `12 B`. */
export function formatBytes(size: number): string {
  if (size < 1024) return `${size} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = size / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`
}

/** One shape for every row, `Sep 2, 2026 18:12`; empty for an unknown time. */
export function formatModified(iso: string | undefined): string {
  if (!iso) return ''
  const time = dayjs(iso)
  return time.isValid() ? time.format('MMM D, YYYY HH:mm') : ''
}

/** The Type column: `Folder`, `TS file`, or `File` when the name has no extension. */
export function entryKind(name: string, isDirectory: boolean): string {
  if (isDirectory) return 'Folder'
  const dot = name.lastIndexOf('.')
  if (dot <= 0 || dot === name.length - 1) return 'File'
  return `${name.slice(dot + 1).toUpperCase()} file`
}
