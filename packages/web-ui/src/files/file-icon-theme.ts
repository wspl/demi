/**
 * The Material Icon Theme assets: the manifest as one lazy chunk, which the page
 * loads in the background once the first conversation has shown
 * (`infra/region-load.ts`) and the first icon shown starts loading if it is not
 * there yet, and each glyph as its own asset the moment a row first asks for
 * it. Nothing here reaches the main bundle until one of them loads.
 */
import { shallowReactive, shallowRef } from 'vue'
import { regionsChanged } from '../infra/region-wait'
import { fileIconThemeSchema, type FileIconTheme } from './file-icons'

const PACKAGE = '../../node_modules/material-icon-theme/'
const glyphs = import.meta.glob<string>('../../node_modules/material-icon-theme/icons/*.svg', {
  query: '?url',
  import: 'default',
})

/** The theme once its manifest has arrived; `null` until then. */
export const fileIconTheme = shallowRef<FileIconTheme | null>(null)
let loading: Promise<void> | undefined

/** Loads the manifest once. */
export function ensureFileIconTheme(): Promise<void> {
  loading ??= import('material-icon-theme/dist/material-icons.json').then((
    module
  ) => {
    fileIconTheme.value = fileIconThemeSchema.parse(module.default)
    regionsChanged()
  })
  return loading
}

const urls = new Map<string, Promise<string | null>>()
/** The pictures that have arrived, by glyph: null for one the package has no file for. */
const pictures = shallowReactive(new Map<string, string | null>())

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

/**
 * The picture of the glyph at `glyph`, which its first ask starts loading:
 * undefined while it is on its way, null where the package has no such file.
 */
export function iconPicture(glyph: string): string | null | undefined {
  if (!pictures.has(glyph) && !urls.has(glyph)) {
    void fileIconUrl(glyph).then((url) => {
      pictures.set(glyph, url)
      regionsChanged()
    })
  }
  return pictures.get(glyph)
}
