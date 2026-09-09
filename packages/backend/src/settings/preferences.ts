import { z } from 'zod'

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
export const preferencesSchema = z.strictObject({
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
    appearance: { ...current.appearance, ...patch.appearance },
    shortcuts
  }
}
