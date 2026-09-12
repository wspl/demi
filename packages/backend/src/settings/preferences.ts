import type { UserPreferences, PreferencesPatch } from '@demicodes/product-contracts'

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
