import { z } from 'zod'
import { commandLocaleSchema } from '@demicodes/command-protocol'

/**
 * Stored overrides only; rendering defaults and keyboard interpretation belong
 * to the browser host.
 */
const appearanceSchema = z.strictObject({
  theme: z.enum(['system', 'light', 'dark']).optional(),
  tone: z.enum(['ink', 'warm']).optional(),
  accent: z.enum([
    'blue',
    'purple',
    'pink',
    'red',
    'orange',
    'green',
    'teal'
  ]).optional(),
  fontSize: z.number().int().min(12).max(18).optional(),
})
const shortcutSchema = z.string().max(64)
/**
 * The time zone and languages the user's browser last reported. The zone must
 * be one this platform knows and every tag well formed; tags are stored in
 * their canonical form.
 */
const localeSchema = commandLocaleSchema.refine(
  locale => knownTimeZone(locale.timeZone),
  { message: 'Unknown time zone', path: ['timeZone'] },
).refine(
  locale => canonicalLanguages(locale.languages) !== null,
  { message: 'Malformed language tag', path: ['languages'] },
).transform(locale => ({
  timeZone: locale.timeZone,
  languages: canonicalLanguages(locale.languages)!,
}))

function knownTimeZone(timeZone: string): boolean {
  try {
    new Intl.DateTimeFormat('en-US', { timeZone })
    return true
  } catch {
    return false
  }
}

/** Canonical tags in the same order, without repeats; null when one is malformed. */
function canonicalLanguages(languages: string[]): string[] | null {
  try {
    return Intl.getCanonicalLocales(languages)
  } catch {
    return null
  }
}
const lastModelSchema = z.strictObject({
  providerId: z.string().min(1),
  modelId: z.string().min(1),
  thinkingEffort: z.string().nullable(),
  serviceTierId: z.string().nullable(),
})
export const preferencesSchema = z.strictObject({
  lastModel: lastModelSchema.optional(),
  locale: commandLocaleSchema.optional(),
  appearance: appearanceSchema,
  shortcuts: z.strictObject({
    new: shortcutSchema.optional(),
    sidebar: shortcutSchema.optional(),
    settings: shortcutSchema.optional()
  }),
})

/**
 * Null clears one shortcut override, so a reset does not replace unrelated
 * settings.
 */
export const preferencesPatchSchema = z.strictObject({
  lastModel: lastModelSchema.optional(),
  locale: localeSchema.optional(),
  appearance: appearanceSchema.optional(),
  shortcuts: z.strictObject({
    new: shortcutSchema.nullable().optional(),
    sidebar: shortcutSchema.nullable().optional(),
    settings: shortcutSchema.nullable().optional(),
  }).optional(),
})
export type UserPreferences = z.infer<typeof preferencesSchema>
export type PreferencesPatch = z.infer<typeof preferencesPatchSchema>

export function patchPreferences(
  current: UserPreferences,
  patch: PreferencesPatch
): UserPreferences {
  const shortcuts = { ...current.shortcuts }
  for (const id of ['new', 'sidebar', 'settings'] as const) {
    const keys = patch.shortcuts?.[id]
    if (keys === null)
      delete shortcuts[id]
    else if (keys !== undefined)
      shortcuts[id] = keys
  }
  return {
    ...current,
    ...(patch.lastModel ? { lastModel: patch.lastModel } : {}),
    ...(patch.locale ? { locale: patch.locale } : {}),
    appearance: { ...current.appearance, ...patch.appearance },
    shortcuts
  }
}
