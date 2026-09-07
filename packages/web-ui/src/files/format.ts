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

/** Today shows the time, this year the day, older the date; empty for an unknown time. */
export function formatModified(iso: string | undefined, now = dayjs()): string {
  if (!iso) return ''
  const time = dayjs(iso)
  if (!time.isValid()) return ''
  if (time.isSame(now, 'day')) return time.format('HH:mm')
  if (time.isSame(now, 'year')) return time.format('MMM D, HH:mm')
  return time.format('YYYY-MM-DD')
}

/** The Type column: `Folder`, `TS file`, or `File` when the name has no extension. */
export function entryKind(name: string, isDirectory: boolean): string {
  if (isDirectory) return 'Folder'
  const dot = name.lastIndexOf('.')
  if (dot <= 0 || dot === name.length - 1) return 'File'
  return `${name.slice(dot + 1).toUpperCase()} file`
}
