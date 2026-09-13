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

const paradigmIdSchema = z.enum([
  'demi',
  'neutral',
  'hairline',
  'carved',
  'overlay',
])
const toneIdSchema = z.enum(['zinc', 'cool', 'warm', 'ink'])
const accentIdSchema = z.literal(PRODUCT_ACCENTS.map((accent) => accent.id))
const densityIdSchema = z.enum(['compact', 'regular', 'comfortable'])
const radiusIdSchema = z.enum(['tight', 'medium', 'soft'])
const shadowIdSchema = z.enum(['hairline', 'soft', 'carved'])

export type ParadigmId = z.infer<typeof paradigmIdSchema>
export type ToneId = z.infer<typeof toneIdSchema>
export type AccentId = ProductAccent
export type DensityId = z.infer<typeof densityIdSchema>
export type RadiusId = z.infer<typeof radiusIdSchema>
export type ShadowId = z.infer<typeof shadowIdSchema>

export const ACCENTS = PRODUCT_ACCENTS

export interface Paradigm {
  id: ParadigmId
  name: string
  summary: string
  tone: ToneId
  density: DensityId
  radius: RadiusId
  shadow: ShadowId
}

export const PARADIGMS: readonly Paradigm[] = [
  {
    id: 'demi',
    name: 'Demi',
    summary: 'Product appearance: Ink, regular density, medium radius and hairline shadows.',
    ...productAppearance
  },
  {
    id: 'neutral',
    name: 'Neutral',
    summary: 'Demi’s own achromatic work surface: near-black / paper layers, carved shadow, 8/12 radius, a blue accent used sparingly.',
    tone: 'zinc',
    density: 'compact',
    radius: 'medium',
    shadow: 'carved',
  },
  {
    id: 'hairline',
    name: 'Hairline',
    summary: 'Cool near-black, 28px hits, 0.5px hairline, almost no drop.',
    tone: 'cool',
    density: 'compact',
    radius: 'tight',
    shadow: 'hairline',
  },
  {
    id: 'carved',
    name: 'Carved',
    summary: 'Warm black, carved shadow, larger radius, session and user bubble on one surface.',
    tone: 'warm',
    density: 'comfortable',
    radius: 'soft',
    shadow: 'carved',
  },
  {
    id: 'overlay',
    name: 'Overlay',
    summary: 'Charcoal plus white washes, double stroke, step-row rhythm.',
    tone: 'ink',
    density: 'regular',
    radius: 'medium',
    shadow: 'carved',
  },
]

const STORAGE_KEY = 'demi-gallery-style'

export interface GalleryState {
  paradigm: ParadigmId | 'custom'
  mode: ThemeMode
  tone: ToneId
  accent: AccentId
  density: DensityId
  radius: RadiusId
  shadow: ShadowId
}

function paradigmById(id: ParadigmId): Paradigm {
  const found = PARADIGMS.find((item) => item.id === id)
  if (!found)
    throw new Error(`unknown paradigm: ${id}`)
  return found
}

function matchesParadigm(state: GalleryState, paradigm: Paradigm): boolean {
  return (
    state.tone === paradigm.tone
    && state.density === paradigm.density
    && state.radius === paradigm.radius
    && state.shadow === paradigm.shadow
  )
}

/**
 * What `persistGalleryState` wrote. The gallery writes the whole state at once,
 * so a record that no longer matches is style, not data: it is dropped whole
 * and the gallery opens on its default paradigm.
 */
const storedGalleryStateSchema = z.object({
  paradigm: z.union([paradigmIdSchema, z.literal('custom')]),
  mode: themeModeSchema,
  tone: toneIdSchema,
  accent: accentIdSchema,
  density: densityIdSchema,
  radius: radiusIdSchema,
  shadow: shadowIdSchema,
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

/** A stored paradigm carries its own axes; only `custom` keeps the stored axes as they are. */
function initialState(): GalleryState {
  const stored = readStored()
  if (stored?.paradigm === 'custom')
    return stored
  const base = paradigmById(stored ? stored.paradigm : 'demi')
  return {
    paradigm: base.id,
    mode: stored?.mode ?? 'dark',
    tone: base.tone,
    accent: stored?.accent ?? DEFAULT_ACCENT,
    density: base.density,
    radius: base.radius,
    shadow: base.shadow,
  }
}

export const galleryState = reactive<GalleryState>(initialState())

export function applyParadigm(id: ParadigmId): void {
  const paradigm = paradigmById(id)
  galleryState.paradigm = paradigm.id
  galleryState.tone = paradigm.tone
  galleryState.density = paradigm.density
  galleryState.radius = paradigm.radius
  galleryState.shadow = paradigm.shadow
}

export function syncParadigmLabel(): void {
  const match = PARADIGMS.find((paradigm) => matchesParadigm(galleryState, paradigm))
  galleryState.paradigm = match?.id ?? 'custom'
}

function writeAttributes(): void {
  const root = document.documentElement
  root.setAttribute('data-theme', galleryState.mode)
  root.setAttribute('data-tone', galleryState.tone)
  root.setAttribute('data-accent', galleryState.accent)
  root.setAttribute('data-density', galleryState.density)
  root.setAttribute('data-radius', galleryState.radius)
  root.setAttribute('data-shadow', galleryState.shadow)
  setTheme(galleryState.mode)
}

export function persistGalleryState(): void {
  writeAttributes()
  localStorage.setItem(STORAGE_KEY, JSON.stringify({
    paradigm: galleryState.paradigm,
    mode: galleryState.mode,
    tone: galleryState.tone,
    accent: galleryState.accent,
    density: galleryState.density,
    radius: galleryState.radius,
    shadow: galleryState.shadow,
  }))
}

watch(
  () => [
    galleryState.tone,
    galleryState.density,
    galleryState.radius,
    galleryState.shadow
  ] as const,
  () => {
    syncParadigmLabel()
  },
)

watch(galleryState, persistGalleryState, { deep: true })

appThemeStore.subscribe(() => {
  if (galleryState.mode !== appThemeStore.state.mode) {
    galleryState.mode = appThemeStore.state.mode
  }
})
