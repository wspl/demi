/**
 * The Material Icon Theme assets, loaded on first use: the manifest as one lazy
 * chunk, each glyph as its own asset the moment a row asks for it. Nothing here
 * reaches the main bundle until a file browser opens.
 */
import { shallowRef } from 'vue'
import { fileIconThemeSchema, type FileIconTheme } from './file-icons'

const PACKAGE = '../../node_modules/material-icon-theme/'
const glyphs = import.meta.glob<string>('../../node_modules/material-icon-theme/icons/*.svg', {
  query: '?url',
  import: 'default',
})

/** The theme once its manifest has arrived; `null` until then. */
export const fileIconTheme = shallowRef<FileIconTheme | null>(null)
let loading: Promise<void> | undefined

export function ensureFileIconTheme(): void {
  loading ??= import('material-icon-theme/dist/material-icons.json').then((
    module
  ) => {
    fileIconTheme.value = fileIconThemeSchema.parse(module.default)
  })
}

const urls = new Map<string, Promise<string | null>>()

/**
 * The asset URL for a glyph at `glyph` in the theme's package
 * (`fileIconGlyph`), or `null` when the package has no such file.
 */
export function fileIconUrl(glyph: string): Promise<string | null> {
  let url = urls.get(glyph)
  if (!url) {
    const load = glyphs[`${PACKAGE}${glyph}`]
    url = load ? load() : Promise.resolve(null)
    urls.set(glyph, url)
  }
  return url
}
