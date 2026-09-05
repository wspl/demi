import { reactive, watch } from 'vue'
import { appThemeStore, setTheme } from '@demicodes/web-ui/theme/appTheme'

type ThemeMode = 'light' | 'dark'

export type ParadigmId = 'neutral' | 'hairline' | 'carved' | 'overlay'
export type ToneId = 'zinc' | 'cool' | 'warm' | 'ink'
export type AccentId = 'steel' | 'indigo' | 'teal' | 'moss' | 'amber' | 'coral' | 'violet'
export type DensityId = 'compact' | 'regular' | 'comfortable'
export type RadiusId = 'tight' | 'medium' | 'soft'
export type ShadowId = 'hairline' | 'soft' | 'carved'

export interface Accent {
  id: AccentId
  name: string
  swatch: string
}

export const ACCENTS: readonly Accent[] = [
  { id: 'steel', name: 'Steel', swatch: 'oklch(58% 0.11 250)' },
  { id: 'indigo', name: 'Indigo', swatch: 'oklch(56% 0.12 285)' },
  { id: 'teal', name: 'Teal', swatch: 'oklch(54% 0.09 200)' },
  { id: 'moss', name: 'Moss', swatch: 'oklch(52% 0.09 145)' },
  { id: 'amber', name: 'Amber', swatch: 'oklch(62% 0.11 75)' },
  { id: 'coral', name: 'Coral', swatch: 'oklch(58% 0.11 28)' },
  { id: 'violet', name: 'Violet', swatch: 'oklch(56% 0.12 305)' },
]

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
    id: 'neutral',
    name: 'Neutral',
    summary: 'Demi’s own achromatic work surface: near-black / paper layers, carved shadow, 8/12 radius, steel accent used sparingly.',
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
  if (!found) throw new Error(`unknown paradigm: ${id}`)
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

interface StoredGalleryState {
  paradigm?: string
  mode?: string
  tone?: ToneId
  accent?: string
  density?: DensityId
  radius?: RadiusId
  shadow?: ShadowId
}

function resolveAccent(id: unknown): AccentId {
  if (typeof id === 'string' && ACCENTS.some((item) => item.id === id)) return id as AccentId
  return 'steel'
}

function readStored(): StoredGalleryState {
  if (typeof localStorage === 'undefined') return {}
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? JSON.parse(raw) as StoredGalleryState : {}
  } catch {
    return {}
  }
}

const stored = readStored()
// A stored paradigm carries its own axes; only `custom` keeps the stored axes as they are.
const custom = stored.paradigm === 'custom'
const base = PARADIGMS.find((item) => item.id === stored.paradigm) ?? paradigmById('neutral')

export const galleryState = reactive<GalleryState>({
  paradigm: custom ? 'custom' : base.id,
  mode: stored.mode === 'light' || stored.mode === 'dark' ? stored.mode : 'dark',
  tone: custom ? stored.tone ?? base.tone : base.tone,
  accent: resolveAccent(stored.accent),
  density: custom ? stored.density ?? base.density : base.density,
  radius: custom ? stored.radius ?? base.radius : base.radius,
  shadow: custom ? stored.shadow ?? base.shadow : base.shadow,
})

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
  () => [galleryState.tone, galleryState.density, galleryState.radius, galleryState.shadow] as const,
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
