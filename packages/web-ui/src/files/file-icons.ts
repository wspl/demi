/**
 * Which Material Icon Theme glyph a name gets. The theme's manifest maps folder
 * names, whole file names and extension suffixes to icon ids; this resolves the
 * way VS Code does, without touching the assets, so it can be tested anywhere.
 */
export interface FileIconTheme {
  /** The icon for a file no rule names. */
  file: string
  /** The icon for a folder no rule names. */
  folder: string
  /** Lower-case folder name → icon id. */
  folderNames: Record<string, string>
  /** Lower-case file name → icon id. */
  fileNames: Record<string, string>
  /** Extension suffix (`ts`, `test.ts`, `d.ts`) → icon id. */
  fileExtensions: Record<string, string>
}

/**
 * A folder matches its name; a file matches its whole name first, then the
 * longest dotted suffix (`app.test.ts` tries `test.ts` before `ts`).
 */
export function fileIconName(theme: FileIconTheme, name: string, isDirectory: boolean): string {
  const lower = name.toLowerCase()
  if (isDirectory) return theme.folderNames[lower] ?? theme.folder
  const whole = theme.fileNames[lower]
  if (whole) return whole
  for (let dot = lower.indexOf('.'); dot !== -1; dot = lower.indexOf('.', dot + 1)) {
    const icon = theme.fileExtensions[lower.slice(dot + 1)]
    if (icon) return icon
  }
  return theme.file
}
