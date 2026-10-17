import { h, type FunctionalComponent } from 'vue'
import VendorMark from './VendorMark.vue'

/** Each mark drawn once, so a row that draws it again keeps its icon. */
const glyphs = new Map<string, FunctionalComponent>()

/**
 * A provider entry's vendor mark as an icon a row takes by component, a menu row's or a
 * dropdown's face: `VendorMark` at a row's size, so an entry looks the same wherever it is drawn.
 */
export function vendorGlyph(label: string, src: string | null): FunctionalComponent {
  const key = `${label}:${src}`
  let glyph = glyphs.get(key)
  if (!glyph) {
    glyph = () => h(VendorMark, { label, src, size: 'row' })
    glyphs.set(key, glyph)
  }
  return glyph
}
