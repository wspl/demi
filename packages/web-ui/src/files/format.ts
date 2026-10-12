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
