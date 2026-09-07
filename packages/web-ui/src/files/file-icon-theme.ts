/**
 * The Material Icon Theme assets, loaded on first use: the manifest as one lazy
 * chunk, each glyph as its own asset the moment a row asks for it. Nothing here
 * reaches the main bundle until a file browser opens.
 */
import { shallowRef } from 'vue'
import type { FileIconTheme } from './file-icons'

const glyphs = import.meta.glob<string>('../../node_modules/material-icon-theme/icons/*.svg', {
  query: '?url',
  import: 'default',
})

/** The theme once its manifest has arrived; `null` until then. */
export const fileIconTheme = shallowRef<FileIconTheme | null>(null)
let loading: Promise<void> | undefined

export function ensureFileIconTheme(): void {
  loading ??= import('material-icon-theme/dist/material-icons.json').then((module) => {
    fileIconTheme.value = module.default as FileIconTheme
  })
}

const urls = new Map<string, Promise<string | null>>()

/** The asset URL for an icon id, or `null` when the theme has no such glyph. */
export function fileIconUrl(icon: string): Promise<string | null> {
  let url = urls.get(icon)
  if (!url) {
    const load = glyphs[`../../node_modules/material-icon-theme/icons/${icon}.svg`]
    url = load ? load() : Promise.resolve(null)
    urls.set(icon, url)
  }
  return url
}
