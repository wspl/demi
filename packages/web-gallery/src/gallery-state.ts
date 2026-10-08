import { reactive, watch } from 'vue'
import { z } from 'zod'
import {
  DEFAULT_ACCENT,
  PRODUCT_ACCENTS,
  productAppearance,
  type ProductAccent
} from '@demicodes/web-ui/theme/productAppearance'
import {
  appThemeStore,
  setTheme,
  themeModeSchema,
  type ThemeMode,
} from '@demicodes/web-ui/theme/appTheme'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'

const paradigmIdSchema = z.enum(['ink', 'warm', 'flat'])
/** The faces on trial for the product, beside the system's. */
const fontIdSchema = z.enum(['system', 'inter', 'geist'])
const accentIdSchema = z.literal(PRODUCT_ACCENTS.map((accent) => accent.id))

export type ParadigmId = z.infer<typeof paradigmIdSchema>
export type AccentId = ProductAccent
export type FontId = z.infer<typeof fontIdSchema>
export const FONT_IDS = fontIdSchema.options

export const ACCENTS = PRODUCT_ACCENTS

/**
 * A theme the gallery can show: the product's two tones, Ink and Warm, and
 * Flat, the one on trial. A theme fixes every token axis but the mode and the
 * accent.
 */
export interface Paradigm {
  id: ParadigmId
  name: string
  summary: SentenceText
  tone: 'ink' | 'warm' | 'paper'
  /** The tone in light mode, when it is not `tone`. */
  lightTone?: 'ink'
  density: 'regular'
  radius: 'medium'
  shadow: 'hairline' | 'flat'
}

export const PARADIGMS: readonly Paradigm[] = [
  {
    id: 'ink',
    name: 'Ink',
    summary: 'The product’s default: neutral grays, regular density, medium radius and hairline shadows.',
    ...productAppearance,
    tone: 'ink',
  },
  {
    id: 'warm',
    name: 'Warm',
    summary: 'The product’s warm tone: the same layout and shadows over warm grays.',
    ...productAppearance,
    tone: 'warm',
  },
  {
    id: 'flat',
    name: 'Flat',
    summary: 'Flat surfaces parted by 0.5px hairlines and a drop only under what floats; dark in measured near-neutral layers, light in Ink’s colours.',
    tone: 'paper',
    lightTone: 'ink',
    density: 'regular',
    radius: 'medium',
    shadow: 'flat',
  },
]

const STORAGE_KEY = 'demi-gallery-style'

export interface GalleryState {
  paradigm: ParadigmId
  mode: ThemeMode
  accent: AccentId
  font: FontId
}

function paradigmById(id: ParadigmId): Paradigm {
  const found = PARADIGMS.find((item) => item.id === id)
  if (!found)
    throw new Error(`unknown paradigm: ${id}`)
  return found
}

/**
 * What `persistGalleryState` wrote. The gallery writes the whole state at once,
 * so a record that no longer matches is style, not data: it is dropped whole
 * and the gallery opens on its default theme.
 */
const storedGalleryStateSchema = z.object({
  paradigm: paradigmIdSchema,
  mode: themeModeSchema,
  accent: accentIdSchema,
  font: fontIdSchema,
})

function readStored(): GalleryState | null {
  if (typeof localStorage === 'undefined')
    return null
  try {
    const stored = storedGalleryStateSchema.safeParse(
      JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null'),
    )
    return stored.success ? stored.data : null
  } catch {
    // Storage may be unavailable, or hold text that is not JSON.
    return null
  }
}

function initialState(): GalleryState {
  return readStored() ?? {
    paradigm: productAppearance.tone,
    mode: 'dark',
    accent: DEFAULT_ACCENT,
    font: 'system',
  }
}

export const galleryState = reactive<GalleryState>(initialState())

export function applyParadigm(id: ParadigmId): void {
  galleryState.paradigm = id
}

function writeAttributes(): void {
  const root = document.documentElement
  const paradigm = paradigmById(galleryState.paradigm)
  root.setAttribute('data-theme', galleryState.mode)
  root.setAttribute('data-tone', galleryState.mode === 'light' ? paradigm.lightTone ?? paradigm.tone : paradigm.tone)
  root.setAttribute('data-accent', galleryState.accent)
  root.setAttribute('data-font', galleryState.font)
  root.setAttribute('data-density', paradigm.density)
  root.setAttribute('data-radius', paradigm.radius)
  root.setAttribute('data-shadow', paradigm.shadow)
  setTheme(galleryState.mode)
}

export function persistGalleryState(): void {
  writeAttributes()
  localStorage.setItem(STORAGE_KEY, JSON.stringify(galleryState))
}

watch(galleryState, persistGalleryState, { deep: true })

appThemeStore.subscribe(() => {
  if (galleryState.mode !== appThemeStore.state.mode) {
    galleryState.mode = appThemeStore.state.mode
  }
})
