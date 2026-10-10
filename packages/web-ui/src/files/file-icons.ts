/**
 * Which Material Icon Theme glyph a name gets. The theme's manifest maps folder
 * names, whole file names and extension suffixes to icon ids; this resolves the
 * way VS Code does, without touching the assets, so it can be tested anywhere.
 */
import { z } from 'zod'
import type { FileBrowserSource } from './types'

/** The parts of the theme's manifest (`dist/material-icons.json`) this reads. */
export const fileIconThemeSchema = z.object({
  /** The icon for a file no rule names. */
  file: z.string(),
  /** The icon for a folder no rule names. */
  folder: z.string(),
  /** Lower-case folder name → icon id. */
  folderNames: z.record(z.string(), z.string()),
  /** Lower-case file name → icon id. */
  fileNames: z.record(z.string(), z.string()),
  /** Extension suffix (`ts`, `test.ts`, `d.ts`) → icon id. */
  fileExtensions: z.record(z.string(), z.string()),
  /**
   * Icon id → its glyph, by a path relative to the manifest's folder: most
   * ids are their own file, some share another's (`instructions` →
   * `./../icons/instructions.clone.svg`).
   */
  iconDefinitions: z.record(z.string(), z.object({ iconPath: z.string() })),
})
export type FileIconTheme = z.infer<typeof fileIconThemeSchema>

/**
 * Where an icon id's glyph is in the theme's package, as its manifest names
 * it (`icons/instructions.clone.svg`); null for an id the theme defines no
 * glyph for.
 */
export function fileIconGlyph(theme: FileIconTheme, icon: string): string | null {
  const iconPath = theme.iconDefinitions[icon]?.iconPath
  // The manifest sits in the package's `dist/`; a URL resolves its `./../`.
  return iconPath ? new URL(iconPath, 'file:///dist/').pathname.slice(1) : null
}

/**
 * A folder matches its name; a file matches its whole name first, then the
 * longest dotted suffix (`app.test.ts` tries `test.ts` before `ts`).
 */
export function fileIconName(
  theme: FileIconTheme,
  name: string,
  isDirectory: boolean
): string {
  const lower = name.toLowerCase()
  if (isDirectory)
    return theme.folderNames[lower] ?? theme.folder
  const whole = theme.fileNames[lower]
  if (whole)
    return whole
  for (let dot = lower.indexOf('.'); dot !== -1; dot = lower.indexOf('.', dot + 1)) {
    const icon = theme.fileExtensions[lower.slice(dot + 1)]
    if (icon)
      return icon
  }
  return theme.file
}

/**
 * The glyph a folder earns by where it sits rather than what it is called: the root
 * wears its operating system's folder (`folder-linux`, `folder-macos`,
 * `folder-windows`), the home directory `folder-home`. Anywhere else, `undefined`
 * and the name decides.
 */
export function landmarkIcon(
  path: string,
  source: Pick<FileBrowserSource, 'platform' | 'home'>
): string | undefined {
  if (path === '/')
    return `folder-${source.platform}`
  if (path === source.home)
    return 'folder-home'
  return undefined
}
